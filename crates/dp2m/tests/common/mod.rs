use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

struct BuildDir(PathBuf);

impl Drop for BuildDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Build firmware once into a private temporary directory via the matching `dp2m`
/// binary (`dp2m.example.fnl`, pinned `SOURCE_DATE_EPOCH`), pass `(out_dir, repo_root)`
/// to `f`, and delete the temporary artifacts before returning (or on unwind).
pub fn with_fresh_build<T>(f: impl FnOnce(&Path, &Path) -> T) -> T {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).expect("system clock").as_nanos();
    let out = BuildDir(std::env::temp_dir().join(format!(
        "dp2m-test-build-{}-{seq}-{nonce}",
        std::process::id()
    )));
    std::fs::create_dir(&out.0).expect("create private test build directory");
    // Cargo supplies the matching builder executable. Never consume a user's out/
    // or local dp2m.fnl: integration suites and Unicorn goldens target dp2m.example.fnl.
    let result = Command::new(env!("CARGO_BIN_EXE_dp2m"))
        .current_dir(&root)
        .args(["build", "--config", "dp2m.example.fnl", "--out"])
        .arg(&out.0)
        .env("SOURCE_DATE_EPOCH", "1767225600")
        .output()
        .expect("run matching dp2m builder (clang and ld.lld must be on PATH)");
    assert!(
        result.status.success(),
        "private test firmware build failed:\n{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    let val = f(&out.0, &root);
    std::fs::remove_dir_all(&out.0).expect("remove private test build artifacts");
    std::mem::forget(out);
    val
}
