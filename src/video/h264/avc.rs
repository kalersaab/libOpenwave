use crate::video::h264::decoder::H264DecodeError;

pub fn parse_avcc(avcc_data: &[u8]) -> Result<(Vec<u8>, u8), H264DecodeError> {
    if avcc_data.len() < 7 {
        return Err(H264DecodeError::InvalidStream);
    }

    let _configuration_version = avcc_data[0];
    let _avc_profile_indication = avcc_data[1];
    let _profile_compatibility = avcc_data[2];
    let _avc_level_indication = avcc_data[3];

    let length_size_minus_one = avcc_data[4] & 0x03;
    let nal_length_size = length_size_minus_one + 1;

    let num_sps = (avcc_data[5] & 0x1F) as usize;
    let mut pos = 6;

    let mut parameter_sets = Vec::new();

    for _ in 0..num_sps {
        if pos + 2 > avcc_data.len() {
            return Err(H264DecodeError::InvalidStream);
        }

        let sps_length = u16::from_be_bytes([avcc_data[pos], avcc_data[pos + 1]]) as usize;
        pos += 2;

        if pos + sps_length > avcc_data.len() {
            return Err(H264DecodeError::InvalidStream);
        }

        parameter_sets.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
        parameter_sets.extend_from_slice(&avcc_data[pos..pos + sps_length]);
        pos += sps_length;
    }

    if pos >= avcc_data.len() {
        return Err(H264DecodeError::InvalidStream);
    }

    let num_pps = avcc_data[pos] as usize;
    pos += 1;

    for _ in 0..num_pps {
        if pos + 2 > avcc_data.len() {
            return Err(H264DecodeError::InvalidStream);
        }

        let pps_length = u16::from_be_bytes([avcc_data[pos], avcc_data[pos + 1]]) as usize;
        pos += 2;

        if pos + pps_length > avcc_data.len() {
            return Err(H264DecodeError::InvalidStream);
        }

        parameter_sets.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
        parameter_sets.extend_from_slice(&avcc_data[pos..pos + pps_length]);
        pos += pps_length;
    }

    Ok((parameter_sets, nal_length_size))
}

pub fn avc1_to_annex_b(
    avc1_data: &[u8],
    nal_length_size: u8,
    include_sps_pps: bool,
    sps_pps: &[u8],
) -> Result<Vec<u8>, H264DecodeError> {
    let mut annex_b = Vec::new();

    if include_sps_pps && !sps_pps.is_empty() {
        annex_b.extend_from_slice(sps_pps);
    }

    let mut pos = 0;

    while pos < avc1_data.len() {
        if pos + nal_length_size as usize > avc1_data.len() {
            return Err(H264DecodeError::InvalidStream);
        }

        let nal_length = match nal_length_size {
            1 => avc1_data[pos] as usize,
            2 => u16::from_be_bytes([avc1_data[pos], avc1_data[pos + 1]]) as usize,
            3 => {
                let bytes = [0, avc1_data[pos], avc1_data[pos + 1], avc1_data[pos + 2]];
                u32::from_be_bytes(bytes) as usize
            }
            4 => {
                let bytes = [
                    avc1_data[pos],
                    avc1_data[pos + 1],
                    avc1_data[pos + 2],
                    avc1_data[pos + 3],
                ];
                u32::from_be_bytes(bytes) as usize
            }
            _ => return Err(H264DecodeError::InvalidStream),
        };

        pos += nal_length_size as usize;

        if pos + nal_length > avc1_data.len() {
            return Err(H264DecodeError::InvalidStream);
        }

        annex_b.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
        annex_b.extend_from_slice(&avc1_data[pos..pos + nal_length]);

        pos += nal_length;
    }

    Ok(annex_b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_avcc_configuration() {
        let avcc = vec![
            0x01, 0x64, 0x00, 0x1F, 0xFF, 0xE1, 0x00, 0x10, 0x67, 0x64, 0x00, 0x1F, 0xAC, 0xD9,
            0x40, 0x50, 0x05, 0xBB, 0x01, 0x6A, 0x02, 0x02, 0x02, 0x80, 0x01, 0x00, 0x05, 0x68,
            0xEB, 0xE3, 0xCB, 0x22,
        ];

        let (parameter_sets, nal_length_size) = parse_avcc(&avcc).unwrap();

        assert_eq!(nal_length_size, 4);
        assert!(!parameter_sets.is_empty());
        assert_eq!(&parameter_sets[0..4], &[0x00, 0x00, 0x00, 0x01]);
    }

    #[test]
    fn converts_avc1_to_annex_b_4_byte() {
        let avc1_data = vec![
            0x00, 0x00, 0x00, 0x05, 0x41, 0x42, 0x43, 0x44, 0x45, 0x00, 0x00, 0x00, 0x03, 0x46,
            0x47, 0x48,
        ];

        let annex_b = avc1_to_annex_b(&avc1_data, 4, false, &[]).unwrap();

        assert_eq!(
            annex_b,
            vec![
                0x00, 0x00, 0x00, 0x01, 0x41, 0x42, 0x43, 0x44, 0x45, 0x00, 0x00, 0x00, 0x01, 0x46,
                0x47, 0x48,
            ]
        );
    }

    #[test]
    fn converts_avc1_to_annex_b_2_byte() {
        let avc1_data = vec![0x00, 0x03, 0x41, 0x42, 0x43];

        let annex_b = avc1_to_annex_b(&avc1_data, 2, false, &[]).unwrap();

        assert_eq!(annex_b, vec![0x00, 0x00, 0x00, 0x01, 0x41, 0x42, 0x43,]);
    }

    #[test]
    fn includes_sps_pps_when_requested() {
        let sps_pps = vec![0x00, 0x00, 0x00, 0x01, 0x67, 0x64, 0x00, 0x1F];

        let avc1_data = vec![0x00, 0x00, 0x00, 0x02, 0x41, 0x42];

        let annex_b = avc1_to_annex_b(&avc1_data, 4, true, &sps_pps).unwrap();

        assert!(annex_b.starts_with(&sps_pps));
        assert!(annex_b.ends_with(&[0x00, 0x00, 0x00, 0x01, 0x41, 0x42]));
    }

    #[test]
    fn rejects_truncated_avc1() {
        let avc1_data = vec![0x00, 0x00, 0x00, 0x10, 0x41, 0x42];

        assert!(avc1_to_annex_b(&avc1_data, 4, false, &[]).is_err());
    }

    #[test]
    fn rejects_invalid_avcc() {
        let avcc = vec![0x01, 0x64];
        assert!(parse_avcc(&avcc).is_err());
    }
}
