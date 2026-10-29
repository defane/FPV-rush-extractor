//! Card in, library out. Port of the Swift `FPVImport` module, extended to
//! action cams: their recordings initially land as whole-file placeholders.
//! Signal segmentation is available for every imported codec.

mod checksum;
mod folders;
mod grouping;
mod importer;
mod ordering;

pub use checksum::sha256;
pub use grouping::{DEFAULT_GAP, by_sequence, by_timestamp};
pub use folders::{FolderPattern, folder_name, session_of_folder, source_label, source_name};
pub use importer::{WHOLE_FILE_ALGORITHM, ImportError, ImportEvent, ImportReport, LibraryImporter, copy_preserving_mtime};
pub use ordering::{DiscoveredFile, GoProChapter, sorted};
