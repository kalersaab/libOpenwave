# libOpenwave

A from-scratch Rust media engine with H.264 video decoding, MP4 container support, AAC audio infrastructure, and WAV playback.

## Features

### ✅ Video Decoding
- **H.264 Baseline Profile Decoder**
  - I-frame, P-frame, B-frame (partial) support
  - CAVLC entropy decoding
  - Motion compensation with sub-macroblock partitions (16x16, 16x8, 8x16, 8x8, 4x4)
  - In-loop deblocking filter
  - Weighted prediction (explicit and implicit)
  - Multiple reference frames
  - Frame/field decoding with MBAFF
  - 168 unit tests covering all components

### ✅ Container Formats
- **MP4 Demuxer**
  - Full box parser (21 box types)
  - Sample extraction with timestamps
  - AVC1→Annex-B NAL conversion
  - Audio and video track support
  - Generic `Mp4Reader<R: Read + Seek>` API

### ✅ Audio
- **AAC Infrastructure** (decoder stub)
  - AudioSpecificConfig parsing from MP4 esds
  - ADTS header parsing for raw streams
  - AAC-LC, Main, SSR, LTP profile detection
  - Multi-channel support (1-7 channels)
  - Frame-based decoding interface
  
- **WAV Playback**
  - Complete WAV parser with streaming
  - PCM format support (8/16/24/32-bit)
  - Audio engine with cpal output
  - Resampling and ring buffer
  - Playback controls (play, pause, seek)

### ✅ Synchronization
- **Audio/Video Sync**
  - Audio clock as master timing source
  - Frame scheduler with PTS-based timing
  - Automatic drift compensation
  - Drop/wait frame decisions
  - Stream state management
  - Seek support with reset
  - 24 comprehensive unit tests

### ✅ Display
- **YUV→RGB Conversion**
  - BT.601 color space conversion
  - 4:2:0 chroma subsampling support
  - PPM image export for frames
  - Efficient pixel-level conversion
  - 4 unit tests

### 🚧 In Progress
- Full AAC-LC audio decoding (IMDCT, bitstream)

### 📋 Planned
- Full AAC-LC audio decoding (IMDCT, Huffman)
- H.264 Main/High profiles
- Video display (SDL2/winit/pixels)
- MKV container support
- Opus audio codec
- Hardware acceleration

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

## Examples

### Decode MP4 and Save Frames

Extract decoded video frames from an MP4 file:

```sh
cargo run --release --example decode_mp4_frames -- input.mp4 10
```

This decodes the first 10 video frames and saves them as PPM images (`frame_0000.ppm`, etc.).

### Play WAV Audio

Use the streaming audio engine for playback with controls:

```sh
cargo run --example play_controls -- openwave-test.wav
```

For seek demonstration:

```sh
cargo run --example test_seek -- openwave-test.wav
```

## Library Overview

### Video
- `openwave::video::h264::decoder::decode_annex_b` - Decode single H.264 picture
- `openwave::video::h264::decoder::decode_annex_b_frames` - Decode multiple pictures
- `openwave::video::h264::avc::avc1_to_annex_b` - Convert AVC1 to Annex-B format
- Supports CAVLC, motion compensation, deblocking, weighted prediction

### Containers
- `openwave::demux::mp4::Mp4Reader` - Parse MP4 files, extract samples
- `openwave::demux::wav::WavFile` - Parse WAV files
- `openwave::demux::wav_stream::WavStreamReader` - Stream WAV with seeking

### Audio
- `openwave::audio::aac::AacDecoder` - AAC decoder interface
- `openwave::audio::decode_integer_pcm` - PCM integer to f32 conversion
- `openwave::audio::engine::AudioEngine` - Streaming audio playback engine
- `openwave::audio::resampler` - Sample rate conversion
- `openwave::audio::ring_buffer` - Lock-free circular buffer

### Synchronization
- `openwave::sync::SyncController` - Audio/video synchronization coordinator
- `openwave::sync::AudioClock` - Sample-based master clock
- `openwave::sync::FrameScheduler` - PTS-based frame timing
- Clock management, drift compensation, seek support

## License

MIT
