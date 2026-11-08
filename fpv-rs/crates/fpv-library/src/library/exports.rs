//! Immutable export snapshots; no database connection crosses to the encoder.
use super::*;
use std::sync::{Arc, Mutex};

pub(super) type ExportLeases = Arc<Mutex<HashMap<Uuid, usize>>>;

#[derive(Debug)]
struct ExportLease {
    sources: Vec<Uuid>,
    counts: ExportLeases,
}
impl Drop for ExportLease {
    fn drop(&mut self) {
        let mut counts = self.counts.lock().unwrap_or_else(|e| e.into_inner());
        for id in &self.sources {
            if let Some(count) = counts.get_mut(id) {
                *count -= 1;
                if *count == 0 {
                    counts.remove(id);
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct PreparedExport {
    parts: Vec<fpv_export::Part>,
    metadata: ExportMetadata,
    tools: Option<PathBuf>,
    mute: bool,
    name: String,
    extension: &'static str,
    _lease: Arc<ExportLease>,
}

#[derive(Debug)]
pub struct PreparedExports {
    flights: Vec<std::result::Result<PreparedExport, (String, String)>>,
}

impl Library {
    pub fn prepare_export(&self, id: &str) -> Result<PreparedExport> {
        let (flight, _, timeline) = self.flight(id)?;
        let session = self.session(&uuid_string(flight.session_id))?;
        let metadata = ExportMetadata {
            session_name: session.name.clone(),
            flying_day: session.date.map(|date| {
                date.with_timezone(&chrono::Local)
                    .format("%Y-%m-%d")
                    .to_string()
            }),
            capture_time: timeline
                .parts
                .first()
                .and_then(|(_, file)| file.media.creation_date)
                .map(|date| date.to_rfc3339()),
        };
        let title = flight
            .title
            .unwrap_or_else(|| format!("Flight {}", flight.order_in_session));
        let (name, extension) = export_name(
            &session.name,
            &title,
            timeline.parts.iter().any(|(_, f)| f.media.is_all_intra()),
        );
        let sources: HashSet<_> = timeline.parts.iter().map(|(_, file)| file.id).collect();
        let mut counts = self.export_leases.lock().unwrap_or_else(|e| e.into_inner());
        for id in &sources {
            *counts.entry(*id).or_default() += 1;
        }
        Ok(PreparedExport {
            parts: timeline.parts,
            metadata,
            tools: self.tools.clone(),
            mute: self.config.mute_videos,
            name,
            extension,
            _lease: Arc::new(ExportLease {
                sources: sources.into_iter().collect(),
                counts: self.export_leases.clone(),
            }),
        })
    }

    pub fn prepare_exports(&self, ids: &[String]) -> Result<PreparedExports> {
        // Check the owner database before accepting a batch; individual missing flights are reported per item.
        self.store.sessions()?;
        Ok(PreparedExports {
            flights: ids
                .iter()
                .map(|id| {
                    self.prepare_export(id)
                        .map_err(|_| (id.clone(), "not in the library".into()))
                })
                .collect(),
        })
    }

    pub(super) fn ensure_export_sources_idle(
        &self,
        ids: impl IntoIterator<Item = Uuid>,
    ) -> Result<()> {
        let counts = self.export_leases.lock().unwrap_or_else(|e| e.into_inner());
        if ids.into_iter().any(|id| counts.contains_key(&id)) {
            return Err(LibraryError::Invalid("Recordings are queued for export or being exported. Wait for the export to finish before modifying their sources.".into()));
        }
        Ok(())
    }
}

impl PreparedExport {
    pub fn write(
        &self,
        destination: &Path,
        mode: ExportMode,
        encoding: ExportEncoding,
    ) -> Result<Vec<PathBuf>> {
        // Text-entry destinations in the native UI have no OS overwrite
        // prompt. Check every part before writing any of them.
        let destinations = if mode == ExportMode::Parts {
            let stem = destination
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy();
            let extension = destination
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or("avi");
            (1..=self.parts.len())
                .map(|index| {
                    destination.with_file_name(format!("{stem} - part{index}.{extension}"))
                })
                .collect()
        } else {
            vec![destination.to_path_buf()]
        };
        if let Some(path) = destinations.iter().find(|path| path.exists()) {
            return Err(LibraryError::Invalid(format!(
                "destination already exists: {}",
                path.display()
            )));
        }
        let ffmpeg = FfmpegRunner::new(self.tools.clone());
        let exporter = ClipExporter::new(&ffmpeg).with_metadata(self.metadata.clone());
        if encoding == ExportEncoding::ProRes422Hq && mode != ExportMode::Parts {
            return Ok(vec![exporter.export_prores(
                &self.parts,
                destination,
                self.mute,
            )?]);
        }
        Ok(match mode {
            ExportMode::Lossless => {
                vec![exporter.export_flight_with_mute(&self.parts, destination, self.mute)?]
            }
            ExportMode::Reencode => vec![exporter.export_flight_reencoded_with_mute(
                &self.parts,
                destination,
                None,
                self.mute,
            )?],
            ExportMode::Parts => {
                let stem = destination
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "flight".into());
                let extension = destination
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("avi");
                self.parts
                    .iter()
                    .enumerate()
                    .map(|(index, (clip, file))| {
                        let path = destination
                            .with_file_name(format!("{stem} - part{}.{extension}", index + 1));
                        let part_metadata = ExportMetadata {
                            capture_time: file.media.creation_date.map(|date| date.to_rfc3339()),
                            ..self.metadata.clone()
                        };
                        let exporter = ClipExporter::new(&ffmpeg).with_metadata(part_metadata);
                        if encoding == ExportEncoding::ProRes422Hq {
                            return Ok(exporter.export_prores(
                                &[(clip.clone(), file.clone())],
                                &path,
                                self.mute,
                            )?);
                        }
                        Ok(exporter.export_clip_with_mute(clip, file, &path, self.mute)?)
                    })
                    .collect::<Result<_>>()?
            }
        })
    }
}

impl PreparedExports {
    pub fn write(
        &self,
        folder: &Path,
        on_mismatch: ExportMode,
        encoding: ExportEncoding,
        mut progress: impl FnMut(usize, usize, &str),
    ) -> Result<BulkExport> {
        std::fs::create_dir_all(folder)?;
        let mut outcome = BulkExport::default();
        for (index, item) in self.flights.iter().enumerate() {
            let export = match item {
                Ok(export) => export,
                Err(error) => {
                    outcome.failed.push(error.clone());
                    continue;
                }
            };
            progress(index, self.flights.len(), &export.name);
            let extension = if encoding == ExportEncoding::ProRes422Hq {
                "mov"
            } else {
                export.extension
            };
            let destination = free_name(folder, &export.name, extension);
            let initial = if encoding == ExportEncoding::ProRes422Hq {
                on_mismatch
            } else {
                ExportMode::Lossless
            };
            let written = match export.write(&destination, initial, encoding) {
                Err(LibraryError::Export(ExportError::MismatchedFrameRates(..))) => {
                    export.write(&destination, on_mismatch, encoding)
                }
                Err(LibraryError::Export(ExportError::NotAllIntra(_)))
                    if on_mismatch == ExportMode::Reencode =>
                {
                    export.write(&destination, ExportMode::Reencode, encoding)
                }
                other => other,
            };
            match written {
                Ok(files) => outcome.written.extend(files),
                Err(error) => outcome
                    .failed
                    .push((export.name.clone(), error.to_string())),
            }
        }
        progress(self.flights.len(), self.flights.len(), "");
        Ok(outcome)
    }
}
