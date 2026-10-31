#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MotionVector {
    pub x: i32,
    pub y: i32,
}

impl MotionVector {
    pub fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }

    pub fn add(&self, other: &Self) -> Option<Self> {
        Some(Self {
            x: self.x.checked_add(other.x)?,
            y: self.y.checked_add(other.y)?,
        })
    }
}

pub struct MotionVectorGrid {
    width: usize,
    height: usize,
    cells: Vec<Option<(u32, MotionVector)>>,
}

impl MotionVectorGrid {
    pub fn new(
        width: usize,
        height: usize,
    ) -> Result<Self, super::super::decoder::H264DecodeError> {
        let length = width
            .checked_mul(height)
            .ok_or(super::super::decoder::H264DecodeError::InvalidStream)?;

        let mut cells = Vec::new();
        cells.try_reserve_exact(length).map_err(|_| {
            super::super::decoder::H264DecodeError::Frame(
                super::super::frame::FrameError::AllocationFailed,
            )
        })?;
        cells.resize(length, None);

        Ok(Self {
            width,
            height,
            cells,
        })
    }

    fn cell(&self, x: usize, y: usize) -> Option<(u32, MotionVector)> {
        (x < self.width && y < self.height)
            .then(|| self.cells[y * self.width + x])
            .flatten()
    }

    fn neighbors(&self, mb_x: u32, mb_y: u32) -> [Option<(u32, MotionVector)>; 3] {
        let x = mb_x as usize * 4;
        let y = mb_y as usize * 4;

        let left = x.checked_sub(1).and_then(|left_x| self.cell(left_x, y + 3));
        let top = y.checked_sub(1).and_then(|top_y| self.cell(x, top_y));
        let top_right = y
            .checked_sub(1)
            .filter(|_| x + 4 < self.width)
            .and_then(|top_y| self.cell(x + 4, top_y));
        let top_left = y
            .checked_sub(1)
            .and_then(|top_y| x.checked_sub(1).and_then(|left_x| self.cell(left_x, top_y)));

        [left, top, top_right.or(top_left)]
    }

    pub fn predict(&self, mb_x: u32, mb_y: u32, reference_index: u32) -> MotionVector {
        let matching = self
            .neighbors(mb_x, mb_y)
            .into_iter()
            .flatten()
            .filter(|(index, _)| *index == reference_index)
            .map(|(_, motion)| motion)
            .collect::<Vec<_>>();

        if matching.len() == 1 {
            return matching[0];
        }

        let mut vectors = self.neighbors(mb_x, mb_y).map(|candidate| {
            candidate
                .filter(|(index, _)| *index == reference_index)
                .map_or(MotionVector::default(), |(_, motion)| motion)
        });

        vectors.sort_by_key(|motion| motion.x);
        let x = vectors[1].x;

        vectors.sort_by_key(|motion| motion.y);
        let y = vectors[1].y;

        MotionVector { x, y }
    }

    pub fn p_skip_requires_zero(&self, mb_x: u32, mb_y: u32) -> bool {
        let [left, top, _] = self.neighbors(mb_x, mb_y);
        [left, top]
            .into_iter()
            .any(|candidate| candidate.is_none_or(|(_, motion)| motion == MotionVector::default()))
    }

    pub fn set_macroblock(
        &mut self,
        mb_x: u32,
        mb_y: u32,
        reference_index: u32,
        motion: MotionVector,
    ) {
        let x = mb_x as usize * 4;
        let y = mb_y as usize * 4;

        for row in 0..4 {
            for column in 0..4 {
                if y + row < self.height && x + column < self.width {
                    self.cells[(y + row) * self.width + x + column] =
                        Some((reference_index, motion));
                }
            }
        }
    }

    pub fn set_block4x4(
        &mut self,
        block_x: usize,
        block_y: usize,
        reference_index: u32,
        motion: MotionVector,
    ) {
        if block_x < self.width && block_y < self.height {
            self.cells[block_y * self.width + block_x] = Some((reference_index, motion));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn predicts_motion_vector_with_median() {
        let mut grid = MotionVectorGrid::new(12, 12).unwrap();

        grid.set_block4x4(3, 7, 0, MotionVector::new(2, 3));

        grid.set_block4x4(4, 3, 0, MotionVector::new(4, 1));

        grid.set_block4x4(8, 3, 0, MotionVector::new(6, 2));

        let predicted = grid.predict(1, 1, 0);

        assert_eq!(predicted, MotionVector::new(4, 2));
    }

    #[test]
    fn uses_single_matching_neighbor() {
        let mut grid = MotionVectorGrid::new(12, 12).unwrap();

        grid.set_block4x4(3, 7, 0, MotionVector::new(5, 7));

        grid.set_block4x4(4, 3, 1, MotionVector::new(1, 1));

        let predicted = grid.predict(1, 1, 0);
        assert_eq!(predicted, MotionVector::new(5, 7));
    }

    #[test]
    fn detects_p_skip_zero_requirement() {
        let mut grid = MotionVectorGrid::new(12, 12).unwrap();

        grid.set_block4x4(4, 3, 0, MotionVector::default());
        assert!(grid.p_skip_requires_zero(1, 1));

        grid.set_block4x4(3, 7, 0, MotionVector::new(1, 1));
        grid.set_block4x4(4, 3, 0, MotionVector::new(2, 2));
        assert!(!grid.p_skip_requires_zero(1, 1));
    }

    #[test]
    fn sets_macroblock_motion() {
        let mut grid = MotionVectorGrid::new(12, 12).unwrap();
        let motion = MotionVector::new(3, -2);

        grid.set_macroblock(0, 1, 0, motion);

        for row in 0..4 {
            for col in 0..4 {
                assert_eq!(grid.cell(col, 4 + row), Some((0, motion)));
            }
        }
    }
}
