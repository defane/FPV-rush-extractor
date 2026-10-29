use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use chrono::{DateTime, Utc};
use fpv_core::{Clip, ClipEntry, ClipExit, ClipKind, DetectionProfile, DetectionRun, FigureTag, Flight, FrameRate,
               MediaInfo, REDUCED_ALGORITHM, Session, SourceFile};
use rusqlite::{Connection, OptionalExtension, Row, params};
use uuid::Uuid;

use crate::codec::{Date, Id};
use crate::schema;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error("could not create the library directory: {0}")]
    Directory(#[from] std::io::Error),
    /// An update aimed at a row that does not exist: corrections move
    /// boundaries, they never invent clips.
    #[error("no {table} with id {id}")]
    NotFound { table: &'static str, id: String },
}

pub type Result<T> = std::result::Result<T, StoreError>;

/// One playback proxy in the cache index. Not a domain type: the files on disk
/// are the truth, and this is only what the cache remembers about them — above
/// all when each was last watched, which the filesystem will not reliably say.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProxyEntry {
    pub key: String,
    pub bytes: i64,
    pub created_at: DateTime<Utc>,
    pub last_used_at: DateTime<Utc>,
}

/// Reading and writing the library. Everything the UI needs, and nothing that
/// knows about ffmpeg or the filesystem layout.
///
/// One connection behind a mutex, as GRDB's `DatabaseQueue` serialises access:
/// a library is one user's, and its writes are a handful per minute.
pub struct LibraryStore {
    connection: Mutex<Connection>,
}

impl LibraryStore {
    /// Opens, creating the file and its directory if needed, and migrates.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        Self::with_connection(Connection::open(path)?, None)
    }

    /// An in-memory library, for tests.
    pub fn in_memory() -> Result<Self> {
        Self::with_connection(Connection::open_in_memory()?, None)
    }

    fn with_connection(mut connection: Connection, up_to: Option<&str>) -> Result<Self> {
        // GRDB turns foreign keys on for every connection; the cascades that
        // clean up after a deleted session depend on it.
        connection.pragma_update(None, "foreign_keys", true)?;
        schema::migrate(&mut connection, up_to)?;
        Ok(Self { connection: Mutex::new(connection) })
    }

    fn db(&self) -> MutexGuard<'_, Connection> {
        // A panic mid-write rolls its transaction back, so the connection
        // behind a poisoned lock is still consistent.
        self.connection.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    // Writing. Saves are upserts that update in place: `INSERT OR REPLACE`
    // would delete the old row first and fire every cascade under it.

    pub fn save_session(&self, session: &Session) -> Result<()> {
        self.db().execute(
            r#"INSERT INTO session (id, name, date, location, notes, detectionProfile)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6)
               ON CONFLICT(id) DO UPDATE SET name = excluded.name, date = excluded.date,
                 location = excluded.location, notes = excluded.notes,
                 detectionProfile = excluded.detectionProfile"#,
            params![Id(session.id), session.name, session.date.map(Date), session.location,
                    session.notes, session.detection_profile.as_str()])?;
        Ok(())
    }

    pub fn save_source_files(&self, files: &[SourceFile]) -> Result<()> {
        let mut db = self.db();
        let transaction = db.transaction()?;
        for file in files {
            upsert_source_file(&transaction, file)?;
        }
        transaction.commit()?;
        Ok(())
    }

    /// Swaps a DVR recording for the pieces it was cut down to, in one
    /// transaction: the pieces arrive, the clips and figure tags move onto
    /// them, each piece gets its run, and the recording's row goes — taking
    /// with it, by cascade, the tags that fell in the snow. A failure leaves
    /// the library as it was.
    pub fn apply_reduction(&self, original: Uuid, pieces: &[SourceFile], clips: &[Clip],
                           tags: &[FigureTag], runs: &[DetectionRun]) -> Result<()> {
        let mut db = self.db();
        let transaction = db.transaction()?;
        for piece in pieces {
            upsert_source_file(&transaction, piece)?;
        }
        for clip in clips {
            let changed = transaction.execute(
                r#"UPDATE clip SET flightId = ?2, sourceFileId = ?3, startFrame = ?4, endFrame = ?5,
                     kind = ?6, entry = ?7, exit = ?8, confidence = ?9, craftName = ?10,
                     thumbnailPath = ?11 WHERE id = ?1"#,
                clip_params(clip))?;
            not_found_unless(changed, "clip", clip.id)?;
        }
        for tag in tags {
            upsert_tag(&transaction, tag)?;
        }
        for run in runs {
            upsert_run(&transaction, run)?;
        }
        transaction.execute("DELETE FROM sourceFile WHERE id = ?", [Id(original)])?;
        transaction.commit()?;
        Ok(())
    }

    /// Removes a flight and, in the same transaction, the recordings given —
    /// those that held nothing but it. Its clips, labels and the recordings'
    /// figure tags and runs go by cascade. The files on disk are the caller's.
    pub fn delete_flight(&self, flight_id: Uuid, files: &[Uuid]) -> Result<()> {
        let mut db = self.db();
        let transaction = db.transaction()?;
        let changed = transaction.execute("DELETE FROM flight WHERE id = ?", [Id(flight_id)])?;
        not_found_unless(changed, "flight", flight_id)?;
        for file in files {
            transaction.execute("INSERT OR IGNORE INTO deletedRecording (path)
                                 SELECT importedPath FROM sourceFile WHERE id = ?", [Id(*file)])?;
            transaction.execute("DELETE FROM sourceFile WHERE id = ?", [Id(*file)])?;
        }
        transaction.commit()?;
        Ok(())
    }

    /// Records where a recording now lives, after it was moved.
    pub fn set_imported_path(&self, file_id: Uuid, path: &str) -> Result<()> {
        let changed = self.db().execute("UPDATE sourceFile SET importedPath = ?2 WHERE id = ?1",
                                        params![Id(file_id), path])?;
        not_found_unless(changed, "source file", file_id)
    }

    /// The session's recordings already cut down to their flights.
    pub fn reduced_files(&self, session_id: Uuid) -> Result<HashSet<Uuid>> {
        let db = self.db();
        let mut statement = db.prepare(
            "SELECT DISTINCT detectionRun.sourceFileId FROM detectionRun
             JOIN sourceFile ON sourceFile.id = detectionRun.sourceFileId
             WHERE sourceFile.sessionId = ?1 AND detectionRun.algorithmVersion = ?2")?;
        let rows = statement.query_map(params![Id(session_id), REDUCED_ALGORITHM],
                                       |row| Ok(row.get::<_, Id>(0)?.0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Replaces a source file's clips with the result of a fresh detection run.
    ///
    /// Re-running the detector must be cheap and reversible (CLAUDE.md 3.2), so
    /// this is one transaction: the old clips and the flights left empty by their
    /// removal go, the new ones and the run that produced them arrive together.
    pub fn replace_clips(&self, file_id: Uuid, clips: &[Clip], flights: &[Flight],
                         run: &DetectionRun) -> Result<()> {
        let mut db = self.db();
        let transaction = db.transaction()?;
        transaction.execute("DELETE FROM clip WHERE sourceFileId = ?", [Id(file_id)])?;
        transaction.execute(
            "DELETE FROM flight WHERE NOT EXISTS (SELECT 1 FROM clip WHERE clip.flightId = flight.id)",
            [])?;
        for flight in flights {
            upsert_flight(&transaction, flight)?;
        }
        for clip in clips {
            upsert_clip(&transaction, clip)?;
        }
        upsert_run(&transaction, run)?;
        transaction.commit()?;
        Ok(())
    }

    /// Persists a hand-corrected clip — boundary edits from the review UI.
    /// The record must already exist.
    pub fn update_clip(&self, clip: &Clip) -> Result<()> {
        let changed = self.db().execute(
            r#"UPDATE clip SET flightId = ?2, sourceFileId = ?3, startFrame = ?4, endFrame = ?5,
                 kind = ?6, entry = ?7, exit = ?8, confidence = ?9, craftName = ?10,
                 thumbnailPath = ?11 WHERE id = ?1"#,
            clip_params(clip))?;
        not_found_unless(changed, "clip", clip.id)
    }

    /// None means no explicit playback preference; use the app's default.
    pub fn playback_mute(&self, flight: Uuid) -> Result<Option<bool>> {
        Ok(self.db().query_row("SELECT muted FROM flightPlayback WHERE flightId = ?",
            [Id(flight)], |row| row.get(0)).optional()?)
    }

    pub fn set_playback_mute(&self, flight: Uuid, muted: bool) -> Result<()> {
        self.db().execute("INSERT INTO flightPlayback (flightId, muted) VALUES (?1, ?2) \
            ON CONFLICT(flightId) DO UPDATE SET muted = excluded.muted", params![Id(flight), muted])?;
        Ok(())
    }

    /// Persists a user edit to a flight — its rating or title.
    pub fn update_flight(&self, flight: &Flight) -> Result<()> {
        let changed = self.db().execute(
            r#"UPDATE flight SET sessionId = ?2, orderInSession = ?3, title = ?4, rating = ?5
               WHERE id = ?1"#,
            params![Id(flight.id), Id(flight.session_id), flight.order_in_session, flight.title,
                    flight.rating])?;
        not_found_unless(changed, "flight", flight.id)
    }

    /// Action-cam files never read for their camera: probed before the
    /// column existed. The DVR's MJPEG names none and is left out.
    pub fn unidentified_cameras(&self) -> Result<Vec<SourceFile>> {
        let db = self.db();
        let mut statement = db.prepare(&format!(
            "SELECT {SOURCE_FILE_COLUMNS} FROM sourceFile \
             WHERE camera IS NULL AND videoCodec != 'mjpeg' ORDER BY importedPath"))?;
        let rows = statement.query_map([], source_file)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Records the camera a file named, or that it named none — so it is not
    /// read again.
    pub fn set_camera(&self, file_id: Uuid, camera: Option<&str>) -> Result<()> {
        let changed = self.db().execute("UPDATE sourceFile SET camera = ?2 WHERE id = ?1",
                                        params![Id(file_id), camera.unwrap_or("")])?;
        not_found_unless(changed, "source file", file_id)
    }

    /// Removes a session and, through the schema's cascades, its source files,
    /// flights, clips, detection runs and tags. The files on disk are the
    /// caller's business — the store never touches the filesystem.
    pub fn delete_session(&self, id: Uuid) -> Result<()> {
        self.db().execute("DELETE FROM session WHERE id = ?", [Id(id)])?;
        Ok(())
    }

    /// Adds a figure tag, or replaces the one with the same ID.
    pub fn save_tag(&self, tag: &FigureTag) -> Result<()> {
        upsert_tag(&self.db(), tag)?;
        Ok(())
    }

    pub fn delete_tag(&self, id: Uuid) -> Result<()> {
        self.db().execute("DELETE FROM figureTag WHERE id = ?", [Id(id)])?;
        Ok(())
    }

    /// Adds or refreshes a playback proxy's row in the cache index.
    pub fn save_proxy(&self, entry: &ProxyEntry) -> Result<()> {
        self.db().execute(
            r#"INSERT INTO proxy (key, bytes, createdAt, lastUsedAt) VALUES (?1, ?2, ?3, ?4)
               ON CONFLICT(key) DO UPDATE SET bytes = excluded.bytes,
                 createdAt = excluded.createdAt, lastUsedAt = excluded.lastUsedAt"#,
            params![entry.key, entry.bytes, Date(entry.created_at), Date(entry.last_used_at)])?;
        Ok(())
    }

    pub fn delete_proxy(&self, key: &str) -> Result<()> {
        self.db().execute("DELETE FROM proxy WHERE key = ?", [key])?;
        Ok(())
    }

    pub fn delete_all_proxies(&self) -> Result<()> {
        self.db().execute("DELETE FROM proxy", [])?;
        Ok(())
    }

    // Reading

    pub fn sessions(&self) -> Result<Vec<Session>> {
        let db = self.db();
        let mut statement = db.prepare(
            "SELECT id, name, date, location, notes, detectionProfile FROM session
             ORDER BY date DESC, name")?;
        let rows = statement.query_map([], |row| Ok(Session {
            id: row.get::<_, Id>(0)?.0,
            name: row.get(1)?,
            date: row.get::<_, Option<Date>>(2)?.map(|d| d.0),
            location: row.get(3)?,
            notes: row.get(4)?,
            // An unknown value (a future profile) degrades to the tuned default.
            detection_profile: DetectionProfile::from_raw(&row.get::<_, String>(5)?)
                .unwrap_or_default(),
        }))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn source_files(&self, session_id: Uuid) -> Result<Vec<SourceFile>> {
        let db = self.db();
        let mut statement = db.prepare(&format!(
            "SELECT {SOURCE_FILE_COLUMNS} FROM sourceFile WHERE sessionId = ?
             ORDER BY sequenceNumber, importedPath"))?;
        let rows = statement.query_map([Id(session_id)], source_file)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Flights with their clips, each flight's clips in playing order — which is
    /// the order of the files they come from, since a flight that spans two files
    /// plays the tail of one and then the head of the next.
    pub fn flights(&self, session_id: Uuid) -> Result<Vec<(Flight, Vec<Clip>)>> {
        let db = self.db();
        // The card order lives on sourceFile, so it is read once and used to
        // sort clips in memory. A session is a handful of files.
        let order: HashMap<Uuid, i64> = db
            .prepare("SELECT id, sequenceNumber FROM sourceFile WHERE sessionId = ?")?
            .query_map([Id(session_id)], |row| {
                Ok((row.get::<_, Id>(0)?.0, row.get::<_, Option<i64>>(1)?.unwrap_or(0)))
            })?
            .collect::<rusqlite::Result<_>>()?;
        let flights: Vec<Flight> = db
            .prepare("SELECT id, sessionId, orderInSession, title, rating FROM flight
                      WHERE sessionId = ? ORDER BY orderInSession")?
            .query_map([Id(session_id)], |row| Ok(Flight {
                id: row.get::<_, Id>(0)?.0,
                session_id: row.get::<_, Id>(1)?.0,
                order_in_session: row.get(2)?,
                title: row.get(3)?,
                rating: row.get(4)?,
            }))?
            .collect::<rusqlite::Result<_>>()?;
        let mut statement = db.prepare(&format!(
            "SELECT {CLIP_COLUMNS} FROM clip WHERE flightId = ?"))?;
        flights.into_iter()
            .map(|flight| {
                let mut clips: Vec<Clip> = statement.query_map([Id(flight.id)], clip)?
                    .collect::<rusqlite::Result<_>>()?;
                clips.sort_by_key(|c| (order.get(&c.source_file_id).copied().unwrap_or(0),
                                       c.start_frame));
                Ok((flight, clips))
            })
            .collect()
    }

    /// One flight with its clips in playing order, wherever it lives.
    pub fn flight(&self, id: Uuid) -> Result<Option<(Flight, Vec<Clip>)>> {
        let session: Option<Id> = self.db()
            .query_row("SELECT sessionId FROM flight WHERE id = ?", [Id(id)], |r| r.get(0))
            .optional()?;
        let Some(Id(session)) = session else { return Ok(None) };
        Ok(self.flights(session)?.into_iter().find(|(flight, _)| flight.id == id))
    }

    pub fn source_file(&self, id: Uuid) -> Result<Option<SourceFile>> {
        Ok(self.db()
            .query_row(&format!("SELECT {SOURCE_FILE_COLUMNS} FROM sourceFile WHERE id = ?"),
                       [Id(id)], source_file)
            .optional()?)
    }

    pub fn clip(&self, id: Uuid) -> Result<Option<Clip>> {
        Ok(self.db()
            .query_row(&format!("SELECT {CLIP_COLUMNS} FROM clip WHERE id = ?"), [Id(id)], clip)
            .optional()?)
    }

    /// Every checksum in the library, for duplicate detection at import time.
    pub fn checksums(&self) -> Result<HashSet<String>> {
        let db = self.db();
        let mut statement = db.prepare(
            "SELECT checksum FROM sourceFile WHERE checksum IS NOT NULL")?;
        let rows = statement.query_map([], |row| row.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Where every recording in the library was copied to: a file on disk
    /// that is not among them belongs to no session.
    /// Where the recordings deleted with a flight were (`delete_flight`).
    pub fn deleted_paths(&self) -> Result<HashSet<String>> {
        let db = self.db();
        let mut statement = db.prepare("SELECT path FROM deletedRecording")?;
        let rows = statement.query_map([], |row| row.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Forgets that these recordings were deleted: they are back.
    pub fn forget_deleted(&self, paths: &[String]) -> Result<()> {
        let mut db = self.db();
        let transaction = db.transaction()?;
        for path in paths {
            transaction.execute("DELETE FROM deletedRecording WHERE path = ?", [path])?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn imported_paths(&self) -> Result<HashSet<String>> {
        let db = self.db();
        let mut statement = db.prepare("SELECT importedPath FROM sourceFile")?;
        let rows = statement.query_map([], |row| row.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn tags_of_source_file(&self, id: Uuid) -> Result<Vec<FigureTag>> {
        let db = self.db();
        let mut statement = db.prepare(&format!(
            "SELECT {TAG_COLUMNS} FROM figureTag WHERE sourceFileId = ?
             ORDER BY startFrame, endFrame"))?;
        let rows = statement.query_map([Id(id)], figure_tag)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// The tags that fall in a clip — overlapping its range in the clip's own
    /// recording. Resolved here rather than stored, so a tag outlives the clip
    /// set it was made against.
    pub fn tags_of_clip(&self, clip: &Clip) -> Result<Vec<FigureTag>> {
        Ok(self.tags_of_source_file(clip.source_file_id)?
            .into_iter()
            .filter(|tag| tag.end_frame >= clip.start_frame && tag.start_frame <= clip.end_frame)
            .collect())
    }

    /// Every tag of a session, in card order.
    pub fn tags_in_session(&self, session_id: Uuid) -> Result<Vec<FigureTag>> {
        let db = self.db();
        let columns = TAG_COLUMNS.split(", ").map(|c| format!("figureTag.{c}"))
            .collect::<Vec<_>>().join(", ");
        let mut statement = db.prepare(&format!(
            "SELECT {columns} FROM figureTag
             JOIN sourceFile ON sourceFile.id = figureTag.sourceFileId
             WHERE sourceFile.sessionId = ?
             ORDER BY sourceFile.sequenceNumber, sourceFile.importedPath,
                      figureTag.startFrame, figureTag.endFrame"))?;
        let rows = statement.query_map([Id(session_id)], figure_tag)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Every label in the library, most used first — the autocomplete vocabulary,
    /// built from use instead of a frozen taxonomy.
    /// Labels a flight; a label it already has, in any case, is left as it was.
    pub fn add_flight_label(&self, flight_id: Uuid, label: &str) -> Result<()> {
        let db = self.db();
        let exists = db.query_row("SELECT 1 FROM flight WHERE id = ?", [Id(flight_id)], |_| Ok(()))
            .optional()?.is_some();
        if !exists {
            return not_found_unless(0, "flight", flight_id);
        }
        db.execute("INSERT OR IGNORE INTO flightLabel (flightId, label) VALUES (?1, ?2)",
                   params![Id(flight_id), label])?;
        Ok(())
    }

    pub fn remove_flight_label(&self, flight_id: Uuid, label: &str) -> Result<()> {
        self.db().execute("DELETE FROM flightLabel WHERE flightId = ?1 AND label = ?2",
                          params![Id(flight_id), label])?;
        Ok(())
    }

    /// Every labelled flight of a session and its labels, in the order given.
    pub fn flight_labels_in_session(&self, session_id: Uuid) -> Result<HashMap<Uuid, Vec<String>>> {
        let db = self.db();
        let mut statement = db.prepare(
            "SELECT flightLabel.flightId, flightLabel.label FROM flightLabel
             JOIN flight ON flight.id = flightLabel.flightId
             WHERE flight.sessionId = ? ORDER BY flightLabel.rowid")?;
        let rows = statement.query_map([Id(session_id)], |row| Ok((row.get::<_, Id>(0)?.0, row.get(1)?)))?;
        let mut labels: HashMap<Uuid, Vec<String>> = HashMap::new();
        for row in rows {
            let (flight, label) = row?;
            labels.entry(flight).or_default().push(label);
        }
        Ok(labels)
    }

    /// Replaces a session's tags, in the order given.
    pub fn set_session_labels(&self, session_id: Uuid, labels: &[String]) -> Result<()> {
        let mut db = self.db();
        let transaction = db.transaction()?;
        transaction.execute("DELETE FROM sessionLabel WHERE sessionId = ?", [Id(session_id)])?;
        for label in labels {
            transaction.execute("INSERT OR IGNORE INTO sessionLabel (sessionId, label) VALUES (?1, ?2)",
                                params![Id(session_id), label])?;
        }
        transaction.commit()?;
        Ok(())
    }

    /// Every tagged session and its tags, in the order given.
    pub fn session_labels(&self) -> Result<HashMap<Uuid, Vec<String>>> {
        let db = self.db();
        let mut statement = db.prepare("SELECT sessionId, label FROM sessionLabel ORDER BY rowid")?;
        let rows = statement.query_map([], |row| Ok((row.get::<_, Id>(0)?.0, row.get(1)?)))?;
        let mut labels: HashMap<Uuid, Vec<String>> = HashMap::new();
        for row in rows {
            let (session, label) = row?;
            labels.entry(session).or_default().push(label);
        }
        Ok(labels)
    }

    /// Every tag in the library — on flights and on sessions alike, one
    /// vocabulary — most used first: the suggestions.
    pub fn flight_label_names(&self) -> Result<Vec<String>> {
        let db = self.db();
        let mut statement = db.prepare(
            // With MIN(n), SQLite takes the bare `label` from that row: the
            // spelling first used names the group. Flights' tags count first.
            "SELECT label, MIN(n) FROM (
               SELECT label, rowid AS n FROM flightLabel
               UNION ALL SELECT label, rowid + 1000000000 AS n FROM sessionLabel)
             GROUP BY label COLLATE NOCASE ORDER BY COUNT(*) DESC, label")?;
        let rows = statement.query_map([], |row| row.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn tag_labels(&self) -> Result<Vec<String>> {
        let db = self.db();
        let mut statement = db.prepare(
            "SELECT label FROM figureTag GROUP BY label
             ORDER BY COUNT(*) DESC, MAX(createdAt) DESC, label")?;
        let rows = statement.query_map([], |row| row.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn proxy(&self, key: &str) -> Result<Option<ProxyEntry>> {
        Ok(self.db()
            .query_row("SELECT key, bytes, createdAt, lastUsedAt FROM proxy WHERE key = ?",
                       [key], proxy_entry)
            .optional()?)
    }

    /// The proxy cache index, least recently used first — eviction order.
    pub fn proxies(&self) -> Result<Vec<ProxyEntry>> {
        let db = self.db();
        let mut statement = db.prepare(
            "SELECT key, bytes, createdAt, lastUsedAt FROM proxy ORDER BY lastUsedAt, key")?;
        let rows = statement.query_map([], proxy_entry)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn detection_runs(&self, source_file_id: Uuid) -> Result<Vec<DetectionRun>> {
        let db = self.db();
        let mut statement = db.prepare(
            "SELECT id, sourceFileId, algorithmVersion, parameters, ranAt FROM detectionRun
             WHERE sourceFileId = ? ORDER BY ranAt DESC")?;
        let rows = statement.query_map([Id(source_file_id)], |row| Ok(DetectionRun {
            id: row.get::<_, Id>(0)?.0,
            source_file_id: row.get::<_, Id>(1)?.0,
            algorithm_version: row.get(2)?,
            // Unreadable parameters cost the explanation, not the run.
            parameters: serde_json::from_str(&row.get::<_, String>(3)?).unwrap_or_default(),
            ran_at: row.get::<_, Date>(4)?.0,
        }))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
}

#[cfg(test)]
impl LibraryStore {
    /// A library migrated only as far as `up_to`, to test upgrades.
    fn in_memory_up_to(up_to: &str) -> Result<Self> {
        Self::with_connection(Connection::open_in_memory()?, Some(up_to))
    }
}

fn not_found_unless(changed: usize, table: &'static str, id: Uuid) -> Result<()> {
    if changed == 0 {
        return Err(StoreError::NotFound { table, id: fpv_core::uuid_string(id) });
    }
    Ok(())
}

fn upsert_flight(db: &Connection, flight: &Flight) -> rusqlite::Result<usize> {
    db.execute(
        r#"INSERT INTO flight (id, sessionId, orderInSession, title, rating)
           VALUES (?1, ?2, ?3, ?4, ?5)
           ON CONFLICT(id) DO UPDATE SET sessionId = excluded.sessionId,
             orderInSession = excluded.orderInSession, title = excluded.title,
             rating = excluded.rating"#,
        params![Id(flight.id), Id(flight.session_id), flight.order_in_session, flight.title,
                flight.rating])
}

fn upsert_clip(db: &Connection, clip: &Clip) -> rusqlite::Result<usize> {
    db.execute(
        r#"INSERT INTO clip (id, flightId, sourceFileId, startFrame, endFrame, kind, entry, exit,
             confidence, craftName, thumbnailPath)
           VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
           ON CONFLICT(id) DO UPDATE SET flightId = excluded.flightId,
             sourceFileId = excluded.sourceFileId, startFrame = excluded.startFrame,
             endFrame = excluded.endFrame, kind = excluded.kind, entry = excluded.entry,
             exit = excluded.exit, confidence = excluded.confidence,
             craftName = excluded.craftName, thumbnailPath = excluded.thumbnailPath"#,
        clip_params(clip))
}

fn upsert_source_file(connection: &Connection, file: &SourceFile) -> rusqlite::Result<()> {
    let media = &file.media;
    connection.execute(
        r#"INSERT INTO sourceFile (id, sessionId, originalPath, importedPath,
             sequenceNumber, width, height, frameRateNumerator, frameRateDenominator,
             frameCount, duration, videoCodec, audioCodec, creationDate, checksum, camera, muted)
           VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)
           ON CONFLICT(id) DO UPDATE SET sessionId = excluded.sessionId,
             originalPath = excluded.originalPath, importedPath = excluded.importedPath,
             sequenceNumber = excluded.sequenceNumber, width = excluded.width,
             height = excluded.height, frameRateNumerator = excluded.frameRateNumerator,
             frameRateDenominator = excluded.frameRateDenominator,
             frameCount = excluded.frameCount, duration = excluded.duration,
             videoCodec = excluded.videoCodec, audioCodec = excluded.audioCodec,
             creationDate = excluded.creationDate, checksum = excluded.checksum,
             camera = excluded.camera, muted = excluded.muted"#,
        params![Id(file.id), Id(file.session_id), file.original_path, file.imported_path,
                file.sequence_number, media.width, media.height,
                media.frame_rate.numerator, media.frame_rate.denominator,
                media.frame_count, media.duration, media.video_codec, media.audio_codec,
                media.creation_date.map(Date), file.checksum, media.camera, file.muted])?;
    Ok(())
}

fn upsert_run(connection: &Connection, run: &DetectionRun) -> rusqlite::Result<()> {
    let parameters = serde_json::to_string(&run.parameters)
        .expect("a map of strings to finite numbers always encodes");
    connection.execute(
        r#"INSERT INTO detectionRun (id, sourceFileId, algorithmVersion, parameters, ranAt)
           VALUES (?1, ?2, ?3, ?4, ?5)
           ON CONFLICT(id) DO UPDATE SET sourceFileId = excluded.sourceFileId,
             algorithmVersion = excluded.algorithmVersion,
             parameters = excluded.parameters, ranAt = excluded.ranAt"#,
        params![Id(run.id), Id(run.source_file_id), run.algorithm_version, parameters,
                Date(run.ran_at)])?;
    Ok(())
}

fn upsert_tag(connection: &Connection, tag: &FigureTag) -> rusqlite::Result<()> {
    connection.execute(
        r#"INSERT INTO figureTag (id, sourceFileId, startFrame, endFrame, label, note, createdAt)
           VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
           ON CONFLICT(id) DO UPDATE SET sourceFileId = excluded.sourceFileId,
             startFrame = excluded.startFrame, endFrame = excluded.endFrame,
             label = excluded.label, note = excluded.note, createdAt = excluded.createdAt"#,
        params![Id(tag.id), Id(tag.source_file_id), tag.start_frame, tag.end_frame, tag.label,
                tag.note, Date(tag.created_at)])?;
    Ok(())
}

fn clip_params(clip: &Clip) -> impl rusqlite::Params + '_ {
    (Id(clip.id), Id(clip.flight_id), Id(clip.source_file_id), clip.start_frame,
     clip.end_frame, clip.kind.as_str(), clip.entry.as_str(), clip.exit.as_str(),
     clip.confidence, clip.craft_name.as_deref(), clip.thumbnail_path.as_deref())
}

const SOURCE_FILE_COLUMNS: &str = "id, sessionId, originalPath, importedPath, sequenceNumber, \
    width, height, frameRateNumerator, frameRateDenominator, frameCount, duration, videoCodec, \
    audioCodec, creationDate, checksum, camera, muted";

fn source_file(row: &Row) -> rusqlite::Result<SourceFile> {
    Ok(SourceFile {
        id: row.get::<_, Id>(0)?.0,
        session_id: row.get::<_, Id>(1)?.0,
        original_path: row.get(2)?,
        imported_path: row.get(3)?,
        sequence_number: row.get(4)?,
        media: MediaInfo {
            width: row.get(5)?,
            height: row.get(6)?,
            frame_rate: FrameRate::new(row.get(7)?, row.get(8)?),
            frame_count: row.get(9)?,
            duration: row.get(10)?,
            video_codec: row.get(11)?,
            audio_codec: row.get(12)?,
            creation_date: row.get::<_, Option<Date>>(13)?.map(|d| d.0),
            // '' is "read, and named nothing" (`set_camera`), which is no camera.
            camera: row.get::<_, Option<String>>(15)?.filter(|c| !c.is_empty()),
        },
        checksum: row.get(14)?,
        muted: row.get(16)?,
    })
}

const CLIP_COLUMNS: &str = "id, flightId, sourceFileId, startFrame, endFrame, kind, entry, exit, \
    confidence, craftName, thumbnailPath";

fn clip(row: &Row) -> rusqlite::Result<Clip> {
    // Unknown raw values degrade the way the Swift records do, rather than
    // making a whole flight unreadable.
    Ok(Clip {
        id: row.get::<_, Id>(0)?.0,
        flight_id: row.get::<_, Id>(1)?.0,
        source_file_id: row.get::<_, Id>(2)?.0,
        start_frame: row.get(3)?,
        end_frame: row.get(4)?,
        kind: ClipKind::from_raw(&row.get::<_, String>(5)?).unwrap_or(ClipKind::Unknown),
        entry: ClipEntry::from_raw(&row.get::<_, String>(6)?).unwrap_or(ClipEntry::Unknown),
        exit: ClipExit::from_raw(&row.get::<_, String>(7)?).unwrap_or(ClipExit::SignalLoss),
        confidence: row.get(8)?,
        craft_name: row.get(9)?,
        thumbnail_path: row.get(10)?,
    })
}

const TAG_COLUMNS: &str = "id, sourceFileId, startFrame, endFrame, label, note, createdAt";

fn figure_tag(row: &Row) -> rusqlite::Result<FigureTag> {
    Ok(FigureTag {
        id: row.get::<_, Id>(0)?.0,
        source_file_id: row.get::<_, Id>(1)?.0,
        start_frame: row.get(2)?,
        end_frame: row.get(3)?,
        label: row.get(4)?,
        note: row.get(5)?,
        created_at: row.get::<_, Date>(6)?.0,
    })
}

fn proxy_entry(row: &Row) -> rusqlite::Result<ProxyEntry> {
    Ok(ProxyEntry {
        key: row.get(0)?,
        bytes: row.get(1)?,
        created_at: row.get::<_, Date>(2)?.0,
        last_used_at: row.get::<_, Date>(3)?.0,
    })
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
