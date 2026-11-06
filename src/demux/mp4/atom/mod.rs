pub mod co64;
pub mod ctts;
pub mod ftyp;
pub mod mdat;
pub mod moof;
pub mod moov;
pub mod stco;
pub mod stsc;
pub mod stss;
pub mod stsz;
pub mod stts;

pub use co64::parse_co64;
pub use ctts::parse_ctts;
pub use ftyp::parse_ftyp;
pub use mdat::MdatBox;
pub use moof::MoofBox;
pub use moov::MoovBox;
pub use stco::parse_stco;
pub use stsc::parse_stsc;
pub use stss::parse_stss;
pub use stsz::parse_stsz;
pub use stts::parse_stts;

use super::parser::skip_bytes;
use super::types::BoxHeader;
use super::{Mp4Error, Mp4Result};
use std::io::{Read, Seek};

pub(crate) fn finish_atom<R: Read + Seek>(reader: &mut R, atom_end: u64) -> Mp4Result<()> {
    let current = reader.stream_position()?;
    if current > atom_end {
        return Err(Mp4Error::InvalidFormat(
            "Atom parser read past atom boundary",
        ));
    }
    if current < atom_end {
        skip_bytes(reader, atom_end - current)?;
    }
    Ok(())
}

pub(crate) fn bounded_atom_end(
    header: &BoxHeader,
    atom_start: u64,
    parent_end: u64,
) -> Mp4Result<u64> {
    let atom_end = atom_start
        .checked_add(header.size)
        .ok_or(Mp4Error::InvalidFormat("Atom size overflow"))?;
    if atom_end > parent_end {
        return Err(Mp4Error::InvalidFormat(
            "Atom size extends beyond parent boundary",
        ));
    }
    Ok(atom_end)
}
