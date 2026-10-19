//! On-disk formats of the coverage directory.
//!
//! Both files start with an eight-byte magic and a one-byte version. A
//! reader that meets an unknown version must refuse the file and name the
//! version it found; it must never guess at the layout.

use alloc::{string::String, sync::Arc, vec::Vec};

use miden_core::{
    Word,
    mast::MastForest,
    serde::{ByteReader, ByteWriter, Deserializable, DeserializationError, Serializable},
};
use miden_mast_package::debug_info::PackageDebugInfo;

use crate::hits::PackageHits;

/// Magic prefix of a package file.
pub const PACKAGE_FILE_MAGIC: &[u8; 8] = b"MCOVPKG\0";
/// Current package file version.
pub const PACKAGE_FILE_VERSION: u8 = 1;
/// File extension of package files.
pub const PACKAGE_FILE_EXTENSION: &str = "pkg";

/// Magic prefix of a run file.
pub const RUN_FILE_MAGIC: &[u8; 8] = b"MCOVRUN\0";
/// Current run file version.
pub const RUN_FILE_VERSION: u8 = 2;
/// File extension of run files.
pub const RUN_FILE_EXTENSION: &str = "run";

/// Everything the reporter needs to know about one package, written once per
/// package into the coverage directory.
///
/// The MAST forest is included so the reporter can classify nodes (split,
/// loop, ...) and find procedure roots without the original package, which
/// in test suites is embedded in a binary and never on disk.
#[derive(Debug, Clone)]
pub struct PackageFile {
    /// Commitment of the MAST forest; identifies the package in run files.
    pub commitment: Word,
    /// The package's MAST forest.
    pub forest: Arc<MastForest>,
    /// The package's debug-info section.
    pub debug_info: Arc<PackageDebugInfo>,
}

impl PackageFile {
    /// File name of this package inside the packages directory.
    pub fn file_name(&self) -> String {
        alloc::format!("{}.{PACKAGE_FILE_EXTENSION}", self.commitment.to_hex())
    }
}

impl Serializable for PackageFile {
    fn write_into<W: ByteWriter>(&self, target: &mut W) {
        target.write_bytes(PACKAGE_FILE_MAGIC);
        target.write_u8(PACKAGE_FILE_VERSION);
        self.commitment.write_into(target);
        self.forest.write_into(target);
        let debug_info = self.debug_info.to_bytes();
        target.write_usize(debug_info.len());
        target.write_bytes(&debug_info);
    }
}

impl Deserializable for PackageFile {
    fn read_from<R: ByteReader>(source: &mut R) -> Result<Self, DeserializationError> {
        read_header(source, *PACKAGE_FILE_MAGIC, PACKAGE_FILE_VERSION, "package")?;
        let commitment = Word::read_from(source)?;
        let forest = MastForest::read_from(source)?;
        let len = source.read_usize()?;
        let bytes = source.read_slice(len)?;
        let debug_info = PackageDebugInfo::read_from_bytes(bytes)?;
        Ok(Self {
            commitment,
            forest: Arc::new(forest),
            debug_info: Arc::new(debug_info),
        })
    }
}

/// How a run ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunOutcome {
    /// The program terminated normally.
    Ok,
    /// The program failed with the given error message.
    Failed(String),
}

/// The hits of one VM execution, written when the execution ends.
#[derive(Debug, Clone)]
pub struct RunFile {
    /// Name of the run: the test name in a test suite, the package name from
    /// the command line.
    pub name: String,
    /// How the run ended.
    pub outcome: RunOutcome,
    /// Number of clock cycles observed.
    pub cycles: u64,
    /// Hits per package, keyed by the package's forest commitment.
    pub packages: Vec<(Word, PackageHits)>,
}

impl Serializable for RunFile {
    fn write_into<W: ByteWriter>(&self, target: &mut W) {
        target.write_bytes(RUN_FILE_MAGIC);
        target.write_u8(RUN_FILE_VERSION);
        write_string(target, &self.name);
        match &self.outcome {
            RunOutcome::Ok => target.write_u8(0),
            RunOutcome::Failed(message) => {
                target.write_u8(1);
                write_string(target, message);
            },
        }
        target.write_u64(self.cycles);
        target.write_usize(self.packages.len());
        for (commitment, hits) in &self.packages {
            commitment.write_into(target);
            hits.write_into(target);
        }
    }
}

impl Deserializable for RunFile {
    fn read_from<R: ByteReader>(source: &mut R) -> Result<Self, DeserializationError> {
        read_header(source, *RUN_FILE_MAGIC, RUN_FILE_VERSION, "run")?;
        let name = read_string(source)?;
        let outcome = match source.read_u8()? {
            0 => RunOutcome::Ok,
            1 => RunOutcome::Failed(read_string(source)?),
            other => {
                return Err(DeserializationError::InvalidValue(alloc::format!(
                    "unknown run outcome tag {other}"
                )));
            },
        };
        let cycles = source.read_u64()?;
        let num_packages = source.read_usize()?;
        let mut packages = Vec::with_capacity(num_packages);
        for _ in 0..num_packages {
            let commitment = Word::read_from(source)?;
            let hits = PackageHits::read_from(source)?;
            packages.push((commitment, hits));
        }
        Ok(Self { name, outcome, cycles, packages })
    }
}

/// Checks the magic and version of a file and refuses anything unexpected.
fn read_header<R: ByteReader>(
    source: &mut R,
    magic: [u8; 8],
    version: u8,
    what: &str,
) -> Result<(), DeserializationError> {
    let found = source.read_slice(magic.len())?;
    if found != magic {
        return Err(DeserializationError::InvalidValue(alloc::format!(
            "not a coverage {what} file: bad magic"
        )));
    }
    let found = source.read_u8()?;
    if found != version {
        return Err(DeserializationError::InvalidValue(alloc::format!(
            "unsupported coverage {what} file version {found}, this build reads version {version}"
        )));
    }
    Ok(())
}

fn write_string<W: ByteWriter>(target: &mut W, value: &str) {
    target.write_usize(value.len());
    target.write_bytes(value.as_bytes());
}

fn read_string<R: ByteReader>(source: &mut R) -> Result<String, DeserializationError> {
    let len = source.read_usize()?;
    let bytes = source.read_slice(len)?;
    String::from_utf8(bytes.to_vec())
        .map_err(|_| DeserializationError::InvalidValue("string is not UTF-8".into()))
}
