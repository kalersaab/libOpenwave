# libOpenwave

libOpenwave is a Rust media-engine project. The current implementation includes
integer PCM WAV parsing and playback, plus foundational H.264 bitstream and
parameter-set parsing.

## Requirements

- Rust and Cargo (edition 2024)
- An audio output device for playback examples
- WAV playback currently requires an output device that supports `f32` samples

## Build and test

```sh
cargo build
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```

## Play a WAV file

Use the streaming audio engine for playback with pause, resume, stop, and seek
controls:

```sh
cargo run --example play_controls -- openwave-test.wav
```

Run the seek demonstration with:

```sh
cargo run --example test_seek -- openwave-test.wav
```

An additional example demonstrates direct playback using `WavFile`:

```sh
cargo run --example play_wav -- openwave-test.wav
```

Pass a path to your own PCM WAV file in place of `openwave-test.wav`.

## Library overview

- `openwave::WavFile` and `openwave::WavInfo` provide in-memory WAV parsing.
- `openwave::audio::decode_integer_pcm` decodes 8-, 16-, 24-, and 32-bit
  integer PCM samples to normalized `f32` values.
- `openwave::audio::engine::AudioEngine` streams WAV audio to the default
  output device and provides playback and seek controls.
- `openwave::demux::wav_stream::WavStreamReader` reads and seeks through WAV
  PCM data in frames.
- `openwave::video::bitstream` and `openwave::video::h264` contain low-level
  bitstream and H.264 parsing utilities. They do not provide full video
  decoding or playback.

## License

MIT
