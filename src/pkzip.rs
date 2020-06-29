#![allow(dead_code)]
#![allow(unused_variables)]

use std::cmp::min;
use std::collections::HashMap;
use std::convert::{TryFrom, TryInto};
use std::fs::File;
use std::io;
use std::io::prelude::*;
use std::mem::size_of;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct Error {
  kind: ErrorKind,
}

#[derive(Debug, Clone)]
pub enum ErrorKind {
  IO(std::io::ErrorKind),
  FromUtf8Error,
  Other,
}

#[repr(u16)]
#[derive(Debug, Copy, Clone)]
pub enum CompressionMethod {
  Uncompressed = 0,
  Deflate = 8,
  Unsupported = u16::MAX,
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub struct ZipArchive<R: Read + io::Seek> {
  reader: R,
  eocd: ZipEndOfCentralDirectory,
  files: Vec<ZipCentralDirectoryFile>,
  names: HashMap<String, usize>,
}

#[derive(Debug)]
pub struct ZipFile {
  header: ZipLocalFileHeader,
}

#[repr(C, packed)]
#[derive(Debug, Copy, Clone)]
struct ZipLocalFileHeader {
  signature: u32,
  min_extract_ver: u16,
  general_purpose_flag: u16,
  compression_method: CompressionMethod,
  last_mod_time: u16,
  last_mod_date: u16,
  crc32: u32,
  compressed_len: u32,
  uncompressed_len: u32,
  file_name_len: u16,
  extra_field_len: u16,
}

#[derive(Debug)]
pub struct ZipCentralDirectoryFile {
  header: ZipCentralDirectoryFileHeader,
  filename: String,
  extra: Vec<u8>,
  comment: String,
}

#[repr(C, packed)]
#[derive(Debug, Copy, Clone)]
struct ZipCentralDirectoryFileHeader {
  signature: u32,
  made_by_ver: u16,
  min_extract_ver: u16,
  general_purpose_flag: u16,
  compression_method: CompressionMethod,
  last_mod_time: u16,
  last_mod_date: u16,
  crc32: u32,
  compressed_len: u32,
  uncompressed_len: u32,
  file_name_len: u16,
  extra_field_len: u16,
  comment_len: u16,
  start_disk: u16,
  internal_attrib: u16,
  external_attrib: u32,
  relative_offset_of_local_header: u32,
}

#[derive(Debug)]
struct ZipEndOfCentralDirectory {
  header: ZipEndOfCentralDirectoryHeader,
  comment: String,
}

#[repr(C, packed)]
#[derive(Debug, Copy, Clone)]
struct ZipEndOfCentralDirectoryHeader {
  signature: u32,
  disk_number: u16,
  start_disk: u16,
  num_disk_entries: u16,
  num_entries: u16,
  central_dir_len: u32,
  cendral_dir_offset: u32,
  comment_len: u16,
}

impl Error {
  pub fn kind(&self) -> &ErrorKind {
    &self.kind
  }
}

impl From<std::io::Error> for Error {
  fn from(error: std::io::Error) -> Self {
    Error {
      kind: ErrorKind::IO(error.kind()),
    }
  }
}

impl From<std::string::FromUtf8Error> for Error {
  fn from(error: std::string::FromUtf8Error) -> Self {
    Error {
      kind: ErrorKind::FromUtf8Error,
    }
  }
}

impl<R: Read + io::Seek> ZipArchive<R> {
  pub fn new(mut reader: R) -> Result<ZipArchive<R>> {
    let eocd = ZipEndOfCentralDirectory::find(&mut reader)?;
    reader.seek(io::SeekFrom::Start(eocd.header.cendral_dir_offset as u64))?;
    let mut files = Vec::with_capacity(eocd.header.num_entries as usize);
    let mut names = HashMap::with_capacity(eocd.header.num_entries as usize);
    for _ in 0..eocd.header.num_entries {
      let cdf = ZipCentralDirectoryFile::find(&mut reader)?;
      names.insert(cdf.filename.clone(), files.len());
      files.push(cdf);
    }
    Ok(ZipArchive {
      reader: reader,
      eocd: eocd,
      files: files,
      names: names,
    })
  }

  pub fn files(&self) -> &Vec<ZipCentralDirectoryFile> {
    &self.files
  }
}

impl ZipArchive<File> {
  pub fn open<P: AsRef<Path>>(path: P) -> Result<ZipArchive<File>> {
    match File::open(path) {
      Ok(file) => Self::new(file),
      _ => Err(Error {
        kind: ErrorKind::Other,
      }),
    }
  }
}

const BUF_SIZE: u64 = 65536;
const LOCAL_FILE_HEADER_SIGNATURE: u32 = 0x04034b50;
const CENTRAL_DIRECTORY_SIGNATURE: u32 = 0x02014b50;
const END_OF_CENTRAL_DIRECTORY_SIGNATURE: u32 = 0x06054b50;

impl ZipEndOfCentralDirectory {
  fn find<R: Read + io::Seek>(reader: &mut R) -> Result<ZipEndOfCentralDirectory> {
    let fsize = reader.seek(io::SeekFrom::End(0))?;
    // check that the file size is at least the minimum size
    if fsize < size_of::<ZipEndOfCentralDirectoryHeader>() as u64 {
      return Err(Error {
        kind: ErrorKind::Other,
      });
    }

    // look for the end of central directory data
    let nbytes = min(fsize, BUF_SIZE);
    reader.seek(io::SeekFrom::Start(fsize - nbytes))?;
    let mut buf = vec![0u8; nbytes as usize];
    reader.read_exact(&mut buf)?;

    // find end of central directory location
    let eocd_pos = match buf.windows(4).rev().position(|window| {
      let array = <[u8; 4]>::try_from(window).unwrap();
      u32::from_le_bytes(array) == END_OF_CENTRAL_DIRECTORY_SIGNATURE
    }) {
      Some(i) => i + 4,
      None => {
        return Err(Error {
          kind: ErrorKind::Other,
        })
      }
    };
    let start = nbytes as usize - eocd_pos;
    let mut bytes = &buf[start..start + size_of::<ZipEndOfCentralDirectoryHeader>()];

    let signature = read_le_u32(&mut bytes);
    let disk_number = read_le_u16(&mut bytes);
    let start_disk = read_le_u16(&mut bytes);
    let num_disk_entries = read_le_u16(&mut bytes);
    let num_entries = read_le_u16(&mut bytes);
    let central_dir_len = read_le_u32(&mut bytes);
    let cendral_dir_offset = read_le_u32(&mut bytes);
    let comment_len = read_le_u16(&mut bytes);
    let start = start + size_of::<ZipEndOfCentralDirectoryHeader>();
    let comment = String::from_utf8(buf[start..start + comment_len as usize].to_vec())?;

    let header = ZipEndOfCentralDirectoryHeader {
      signature: signature,
      disk_number: disk_number,
      start_disk: start_disk,
      num_disk_entries: num_disk_entries,
      num_entries: num_entries,
      central_dir_len: central_dir_len,
      cendral_dir_offset: cendral_dir_offset,
      comment_len: comment_len,
    };

    Ok(ZipEndOfCentralDirectory {
      header: header,
      comment: comment,
    })
  }
}

impl ZipCentralDirectoryFile {
  fn find<R: Read + io::Seek>(reader: &mut R) -> Result<ZipCentralDirectoryFile> {
    let mut buf = [0u8; size_of::<ZipCentralDirectoryFileHeader>()];
    reader.read_exact(&mut buf)?;
    let mut bytes = &buf[..];

    let signature = read_le_u32(&mut bytes);
    if signature != CENTRAL_DIRECTORY_SIGNATURE {
      return Err(Error {
        kind: ErrorKind::Other,
      });
    }
    let made_by_ver = read_le_u16(&mut bytes);
    let min_extract_ver = read_le_u16(&mut bytes);
    let general_purpose_flag = read_le_u16(&mut bytes);
    let compression_method = read_le_u16(&mut bytes);
    let last_mod_time = read_le_u16(&mut bytes);
    let last_mod_date = read_le_u16(&mut bytes);
    let crc32 = read_le_u32(&mut bytes);
    let compressed_len = read_le_u32(&mut bytes);
    let uncompressed_len = read_le_u32(&mut bytes);
    let file_name_len = read_le_u16(&mut bytes);
    let extra_field_len = read_le_u16(&mut bytes);
    let comment_len = read_le_u16(&mut bytes);
    let start_disk = read_le_u16(&mut bytes);
    let internal_attrib = read_le_u16(&mut bytes);
    let external_attrib = read_le_u32(&mut bytes);
    let relative_offset_of_local_header = read_le_u32(&mut bytes);

    let header = ZipCentralDirectoryFileHeader {
      signature: signature,
      made_by_ver: made_by_ver,
      min_extract_ver: min_extract_ver,
      general_purpose_flag: general_purpose_flag,
      compression_method: compression_method.into(),
      last_mod_time: last_mod_time,
      last_mod_date: last_mod_date,
      crc32: crc32,
      compressed_len: compressed_len,
      uncompressed_len: uncompressed_len,
      file_name_len: file_name_len,
      extra_field_len: extra_field_len,
      comment_len: comment_len,
      start_disk: start_disk,
      internal_attrib: internal_attrib,
      external_attrib: external_attrib,
      relative_offset_of_local_header: relative_offset_of_local_header,
    };

    let mut buf =
      vec![0u8; file_name_len as usize + extra_field_len as usize + comment_len as usize];
    reader.read_exact(&mut buf)?;

    let mut start = 0;
    let mut end = file_name_len as usize;
    let filename = match file_name_len {
      0 => String::new(),
      _ => String::from_utf8(buf[start..end].to_vec())?,
    };
    start = end;
    end = start + extra_field_len as usize;
    let extra = match extra_field_len {
      0 => Vec::new(),
      _ => buf[start..end].to_vec(),
    };
    start = end;
    end = start + comment_len as usize;
    let comment = match comment_len {
      0 => String::new(),
      _ => String::from_utf8(buf[start..end].to_vec())?,
    };

    Ok(ZipCentralDirectoryFile {
      header: header,
      filename: filename,
      extra: extra,
      comment: comment,
    })
  }

  pub fn filename(&self) -> &String {
    &self.filename
  }
}

impl From<u16> for CompressionMethod {
  fn from(n: u16) -> CompressionMethod {
    match n {
      0 => CompressionMethod::Uncompressed,
      8 => CompressionMethod::Deflate,
      _ => CompressionMethod::Unsupported,
    }
  }
}

fn read_le_u16(input: &mut &[u8]) -> u16 {
  let (int_bytes, rest) = input.split_at(size_of::<u16>());
  *input = rest;
  u16::from_le_bytes(int_bytes.try_into().unwrap())
}

fn read_le_u32(input: &mut &[u8]) -> u32 {
  let (int_bytes, rest) = input.split_at(size_of::<u32>());
  *input = rest;
  u32::from_le_bytes(int_bytes.try_into().unwrap())
}

fn read_le_u64(input: &mut &[u8]) -> u64 {
  let (int_bytes, rest) = input.split_at(size_of::<u64>());
  *input = rest;
  u64::from_le_bytes(int_bytes.try_into().unwrap())
}
