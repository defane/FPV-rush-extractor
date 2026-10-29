//! The application layer: everything the native GUI does, minus the window.
//! Domain operations are tested without a graphics or audio device.

mod cards;
mod config;
mod library;
mod progress;
pub mod reduce;
mod tools;
mod views;

pub use cards::{Card, detect_cards, recordings_folder};
pub use config::{APP_DIRECTORY, Config};
pub use library::{BulkExport, CacheKind, CacheUsage, DownloadStep, DownloadSummary, ExportMode, FreeUpSummary, FlightDeletion, RelocationPreview, RelocationSummary, SessionEdit, FolderView, ImportSummary, ReducePreview, ReduceSummary, Library, LibraryError, Playback, Segment, StripFrame, StripsPrepared,
                  Thumbnail};
pub use progress::{ImportProgress, SegmentProgress, Step, strip_line};
pub use tools::{ffmpeg_available, tool_directory};
pub use views::{ClipView, FileView, FlightView, SessionView, TagView};
pub use library::{NativeSegment, RecordingInfo, export_name};
