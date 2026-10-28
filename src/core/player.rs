#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayerState {
    Empty,
    Ready,
    Playing,
    Paused,
    Stopped,
}

#[derive(Debug)]
pub struct Player {
    state: PlayerState,
}

impl Default for Player {
    fn default() -> Self {
        Self::new()
    }
}

impl Player {
    pub fn new() -> Self {
        Self {
            state: PlayerState::Empty,
        }
    }

    pub fn state(&self) -> PlayerState {
        self.state
    }

    pub fn load(&mut self) {
        self.state = PlayerState::Ready;
    }

    pub fn play(&mut self) {
        if matches!(
            self.state,
            PlayerState::Ready | PlayerState::Paused | PlayerState::Stopped
        ) {
            self.state = PlayerState::Playing;
        }
    }

    pub fn pause(&mut self) {
        if self.state == PlayerState::Playing {
            self.state = PlayerState::Paused;
        }
    }

    pub fn stop(&mut self) {
        if self.state != PlayerState::Empty {
            self.state = PlayerState::Stopped;
        }
    }

    pub fn unload(&mut self) {
        self.state = PlayerState::Empty;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn player_state_transitions() {
        let mut player = Player::new();
        assert_eq!(player.state(), PlayerState::Empty);

        player.load();
        assert_eq!(player.state(), PlayerState::Ready);

        player.play();
        assert_eq!(player.state(), PlayerState::Playing);

        player.pause();
        assert_eq!(player.state(), PlayerState::Paused);

        player.play();
        assert_eq!(player.state(), PlayerState::Playing);

        player.stop();
        assert_eq!(player.state(), PlayerState::Stopped);

        player.unload();
        assert_eq!(player.state(), PlayerState::Empty);
    }
}
