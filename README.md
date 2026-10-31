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
- `openwave::video::h264::decoder::decode_annex_b` decodes the first supported
  picture from an Annex B H.264 stream.
- `openwave::video::h264::decoder::decode_annex_b_frames` decodes supported
  pictures in bitstream order. The current experimental decoder handles
  CAVLC 8-bit 4:2:0 IDR Intra16x16, P-skip, zero-residual P_L0_16x16
  macroblocks, and limited all-direct-skip B pictures. P_L0_16x16 supports
  quarter-pel luma and bilinear chroma interpolation with edge extension.
  General P/B macroblock syntax, deblocking, and presentation reordering are
  not implemented; this is not a general-purpose video player.

## License

MIT
