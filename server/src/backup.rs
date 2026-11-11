//! Daily SQLite snapshots: `VACUUM INTO <data_dir>/backups/swarmboard-<UTC time>.db`, newest
//! `BACKUP_KEEP` kept (default 14, 0 = off). Checked hourly, so restarts never skip or stack a day.
//! Snapshots are consistent copies taken while the server runs; restore by stopping the server and
//! copying one over the database file.

use std::{
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use sqlx::SqlitePool;

const PREFIX: &str = "swarmboard-";
const EVERY: Duration = Duration::from_secs(24 * 60 * 60);
const CHECK: Duration = Duration::from_secs(60 * 60);

pub fn spawn(db: SqlitePool, dir: PathBuf, keep: usize) {
    if keep == 0 {
        tracing::info!("database backups off (BACKUP_KEEP=0)");
        return;
    }
    tokio::spawn(async move {
        // Let startup and migrations settle before the first snapshot.
        tokio::time::sleep(Duration::from_secs(30)).await;
        loop {
            match run_if_due(&db, &dir, keep, SystemTime::now()).await {
                Ok(Some(path)) => tracing::info!(path = %path.display(), "database backup written"),
                Ok(None) => {}
                Err(e) => tracing::error!(error = %e, "database backup failed"),
            }
            tokio::time::sleep(CHECK).await;
        }
    });
}

/// Takes a snapshot when the newest one is at least a day old (or there is none), then prunes.
pub async fn run_if_due(db: &SqlitePool, dir: &Path, keep: usize, now: SystemTime) -> anyhow::Result<Option<PathBuf>> {
    tokio::fs::create_dir_all(dir).await?;
    let existing = list(dir).await?;
    let due = match existing.last() {
        None => true,
        Some(newest) => {
            let modified = tokio::fs::metadata(newest).await?.modified()?;
            now.duration_since(modified).unwrap_or_default() >= EVERY
        }
    };
    let written = if due { Some(snapshot(db, dir, now).await?) } else { None };
    prune(dir, keep).await?;
    Ok(written)
}

async fn snapshot(db: &SqlitePool, dir: &Path, now: SystemTime) -> anyhow::Result<PathBuf> {
    let ms = now.duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0);
    let path = dir.join(format!("{PREFIX}{}.db", utc_stamp(ms)));
    let tmp = path.with_extension("db.partial");
    let _ = tokio::fs::remove_file(&tmp).await;
    let target = tmp.to_str().ok_or_else(|| anyhow::anyhow!("backup path is not UTF-8"))?;
    sqlx::query("VACUUM INTO ?").bind(target).execute(db).await?;
    tokio::fs::rename(&tmp, &path).await?;
    Ok(path)
}

/// Snapshot files, oldest first (names sort by time).
async fn list(dir: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    let mut rd = tokio::fs::read_dir(dir).await?;
    while let Some(e) = rd.next_entry().await? {
        let name = e.file_name();
        let name = name.to_string_lossy();
        if name.starts_with(PREFIX) && name.ends_with(".db") {
            out.push(e.path());
        }
    }
    out.sort();
    Ok(out)
}

async fn prune(dir: &Path, keep: usize) -> anyhow::Result<()> {
    let files = list(dir).await?;
    let excess = files.len().saturating_sub(keep);
    for old in &files[..excess] {
        tokio::fs::remove_file(old).await?;
    }
    Ok(())
}

/// `20261110T034517Z` for a unix time in ms.
fn utc_stamp(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Days since 1970-01-01 → civil date (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}{m:02}{d:02}T{:02}{:02}{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::random_token;

    #[test]
    fn stamps() {
        assert_eq!(utc_stamp(0), "19700101T000000Z");
        assert_eq!(utc_stamp(1_762_746_317_000), "20251110T034517Z");
        assert_eq!(utc_stamp(951_782_400_000), "20000229T000000Z");
    }

    #[tokio::test]
    async fn snapshots_daily_and_prunes() {
        // A file database, like production: VACUUM INTO from sqlx's in-memory DB stays in memory.
        let root = std::env::temp_dir().join(format!("swarmboard-backup-test-{}", random_token(8)));
        std::fs::create_dir_all(&root).unwrap();
        let opts = sqlx::sqlite::SqliteConnectOptions::new().filename(root.join("live.db")).create_if_missing(true);
        let db = SqlitePool::connect_with(opts).await.unwrap();
        sqlx::query("CREATE TABLE probe (v TEXT)").execute(&db).await.unwrap();
        sqlx::query("INSERT INTO probe VALUES ('kept')").execute(&db).await.unwrap();
        let dir = root.join("backups");

        let first = run_if_due(&db, &dir, 2, SystemTime::now()).await.unwrap().expect("first run snapshots");
        assert!(run_if_due(&db, &dir, 2, SystemTime::now()).await.unwrap().is_none(), "not due again within a day");

        // The snapshot is a complete, openable database.
        let opts = sqlx::sqlite::SqliteConnectOptions::new().filename(&first);
        let copy = sqlx::SqlitePool::connect_with(opts).await.unwrap();
        let v: String = sqlx::query_scalar("SELECT v FROM probe").fetch_one(&copy).await.unwrap();
        assert_eq!(v, "kept");
        copy.close().await;

        // Older snapshots beyond `keep` are removed, newest first kept.
        for name in ["swarmboard-20200101T000000Z.db", "swarmboard-20200102T000000Z.db", "unrelated.db"] {
            std::fs::write(dir.join(name), b"x").unwrap();
        }
        let later = SystemTime::now() + EVERY;
        run_if_due(&db, &dir, 2, later).await.unwrap();
        let names: Vec<String> = list(&dir).await.unwrap().iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(names.len(), 2, "{names:?}");
        assert!(!names.iter().any(|n| n.starts_with("swarmboard-2020")), "{names:?}");
        assert!(dir.join("unrelated.db").exists());
        db.close().await;
        std::fs::remove_dir_all(&root).unwrap();
    }
}
