//! The schema, migration for migration as GRDB applied it — the DDL below is
//! what GRDB 7 generated for the Swift migrator, read back from a real library.
//!
//! The ledger is GRDB's own `grdb_migrations` table, kept so a library the
//! macOS app migrated is recognised as up to date. A migration that has
//! shipped is frozen: a real library already ran it, and an edited migration
//! does not re-run. Every schema change is a new entry at the end.
//!
//! The schema keeps `flight` and `clip` as separate tables on purpose: the DVR
//! cuts a recording mid-flight and the feed continues in the next file, so a
//! flight owns one clip per file it spans (CLAUDE.md 3.3).

use rusqlite::{Connection, OptionalExtension};

pub(crate) const MIGRATIONS: &[(&str, &str)] = &[
    ("v1", r#"
        CREATE TABLE "session" ("id" TEXT PRIMARY KEY NOT NULL, "name" TEXT NOT NULL, "date" DATETIME, "location" TEXT, "notes" TEXT);
        CREATE TABLE "sourceFile" ("id" TEXT PRIMARY KEY NOT NULL, "sessionId" TEXT NOT NULL REFERENCES "session"("id") ON DELETE CASCADE, "originalPath" TEXT NOT NULL, "importedPath" TEXT NOT NULL UNIQUE, "sequenceNumber" INTEGER, "width" INTEGER NOT NULL, "height" INTEGER NOT NULL, "frameRateNumerator" INTEGER NOT NULL, "frameRateDenominator" INTEGER NOT NULL, "frameCount" INTEGER, "duration" DOUBLE NOT NULL, "videoCodec" TEXT NOT NULL, "audioCodec" TEXT, "creationDate" DATETIME, "checksum" TEXT);
        CREATE INDEX "sourceFile_on_sessionId" ON "sourceFile"("sessionId");
        CREATE TABLE "flight" ("id" TEXT PRIMARY KEY NOT NULL, "sessionId" TEXT NOT NULL REFERENCES "session"("id") ON DELETE CASCADE, "orderInSession" INTEGER NOT NULL, "title" TEXT, "rating" INTEGER);
        CREATE INDEX "flight_on_sessionId" ON "flight"("sessionId");
        CREATE TABLE "clip" ("id" TEXT PRIMARY KEY NOT NULL, "flightId" TEXT NOT NULL REFERENCES "flight"("id") ON DELETE CASCADE, "sourceFileId" TEXT NOT NULL REFERENCES "sourceFile"("id") ON DELETE CASCADE, "startFrame" INTEGER NOT NULL, "endFrame" INTEGER NOT NULL, "kind" TEXT NOT NULL, "entry" TEXT NOT NULL, "exit" TEXT NOT NULL, "confidence" DOUBLE NOT NULL, "craftName" TEXT, "thumbnailPath" TEXT);
        CREATE INDEX "clip_on_flightId" ON "clip"("flightId");
        CREATE INDEX "clip_on_sourceFileId" ON "clip"("sourceFileId");
        CREATE TABLE "detectionRun" ("id" TEXT PRIMARY KEY NOT NULL, "sourceFileId" TEXT NOT NULL REFERENCES "sourceFile"("id") ON DELETE CASCADE, "algorithmVersion" TEXT NOT NULL, "parameters" TEXT NOT NULL, "ranAt" DATETIME NOT NULL);
        CREATE INDEX "detectionRun_on_sourceFileId" ON "detectionRun"("sourceFileId");
    "#),
    // Figure tags hang off the recording, not the clip: re-segmenting deletes
    // every clip of a file, and a cascade from there would take the user's tags
    // with it. The proxy cache's access time lives here because atime is
    // commonly off.
    ("v2", r#"
        CREATE TABLE "figureTag" ("id" TEXT PRIMARY KEY NOT NULL, "sourceFileId" TEXT NOT NULL REFERENCES "sourceFile"("id") ON DELETE CASCADE, "startFrame" INTEGER NOT NULL, "endFrame" INTEGER NOT NULL, "label" TEXT NOT NULL, "note" TEXT, "createdAt" DATETIME NOT NULL);
        CREATE INDEX "figureTag_on_sourceFileId" ON "figureTag"("sourceFileId");
        CREATE TABLE "proxy" ("key" TEXT PRIMARY KEY NOT NULL, "bytes" INTEGER NOT NULL, "createdAt" DATETIME NOT NULL, "lastUsedAt" DATETIME NOT NULL);
    "#),
    // The detector profile a session's DVR footage is segmented with
    // (2026-10-19). Every existing session keeps the tuned thresholds.
    ("v3", r#"
        ALTER TABLE "session" ADD COLUMN "detectionProfile" TEXT NOT NULL DEFAULT 'standard';
    "#),
    // The camera a file names in its tags (2026-10-21). NULL for files probed
    // before: the library reads them again once (`identify_cameras`), and ''
    // records that a file was read and named nothing.
    ("v4", r#"
        ALTER TABLE "sourceFile" ADD COLUMN "camera" TEXT;
    "#),
    // Free labels on a whole flight — "freestyle", "gates" (2026-10-23), beside
    // the figure tags that mark a moment in a recording. They hang off the
    // flight like its title and like, and go with it; one label once per
    // flight whatever its case.
    ("v5", r#"
        CREATE TABLE "flightLabel" ("flightId" TEXT NOT NULL REFERENCES "flight"("id") ON DELETE CASCADE, "label" TEXT NOT NULL COLLATE NOCASE, PRIMARY KEY ("flightId", "label"));
    "#),
    // Tags on a whole session (2026-10-24): every flight of it carries them,
    // beside its own. Kept apart from the flights' so a re-segment, which
    // remakes the flights, leaves them.
    ("v6", r#"
        CREATE TABLE "sessionLabel" ("sessionId" TEXT NOT NULL REFERENCES "session"("id") ON DELETE CASCADE, "label" TEXT NOT NULL COLLATE NOCASE, PRIMARY KEY ("sessionId", "label"));
    "#),
    // Recordings that left the library with a deleted flight (2026-10-25), by
    // path: loading the import folder must not bring one back unasked, should
    // the trash have refused it or the pilot put it back.
    ("v7", r#"
        CREATE TABLE "deletedRecording" ("path" TEXT PRIMARY KEY NOT NULL);
    "#),
    // A per-import audio policy, independent of the recording's actual audio codec.
    ("v8", r#"
        ALTER TABLE "sourceFile" ADD COLUMN "muted" INTEGER NOT NULL DEFAULT 0 CHECK ("muted" IN (0, 1));
    "#),
];

/// Applies every migration not yet in the ledger, up to and including
/// `up_to` (all of them when `None`), each in its own transaction.
pub(crate) fn migrate(connection: &mut Connection, up_to: Option<&str>) -> rusqlite::Result<()> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS grdb_migrations (identifier TEXT NOT NULL PRIMARY KEY)")?;
    for &(identifier, sql) in MIGRATIONS {
        let applied = connection
            .query_row("SELECT 1 FROM grdb_migrations WHERE identifier = ?", [identifier],
                       |_| Ok(()))
            .optional()?
            .is_some();
        if !applied {
            let transaction = connection.transaction()?;
            transaction.execute_batch(sql)?;
            transaction.execute("INSERT INTO grdb_migrations (identifier) VALUES (?)",
                                [identifier])?;
            transaction.commit()?;
        }
        if up_to == Some(identifier) {
            break;
        }
    }
    Ok(())
}
