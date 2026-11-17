//! File manager backend, sandboxed to a few roots.
//!
//! Every path is resolved through `canonicalize` (parent for the entry itself, the full
//! path for anything that reads or writes content), so `..` and symlinks cannot leave a
//! root. Overwrites keep the previous version in `<data>/history`, deletes move into
//! `<data>/trash`; both live in the agent's data dir, which is never writable through
//! these methods.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;

use anyhow::{bail, Context, Result};
use base64::Engine;
use styk_proto::{FsChunk, FsEntry, FsListing, FsVersion};

use crate::util::env_list;

const CHUNK_MAX: u32 = 1 << 20;
const WRITE_MAX: usize = 10 << 20;
const LIST_MAX: usize = 5000;
const HISTORY_KEEP: usize = 30;
const HISTORY_MAX_SIZE: u64 = 20 << 20;
const TRASH_DAYS: i64 = 30;

const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;

pub struct Sandbox {
    roots: Vec<(PathBuf, bool)>,
    data: PathBuf,
}

pub fn sandbox() -> &'static Sandbox {
    static S: OnceLock<Sandbox> = OnceLock::new();
    S.get_or_init(|| {
        let roots = env_list("STYK_AGENT_FS_ROOTS", "/opt,/srv,/var/backups:ro")
            .into_iter()
            .map(|r| match r.strip_suffix(":ro") {
                Some(p) => (PathBuf::from(p), false),
                None => (PathBuf::from(r), true),
            })
            .collect();
        let data =
            std::env::var("STYK_AGENT_DATA_DIR").unwrap_or_else(|_| "/opt/styk-panel/data".into());
        Sandbox::new(roots, PathBuf::from(data))
    })
}

struct Resolved {
    /// Canonical parent joined with the entry name (the entry itself is not followed).
    path: PathBuf,
    root: PathBuf,
    writable: bool,
}

impl Sandbox {
    pub fn new(roots: Vec<(PathBuf, bool)>, data: PathBuf) -> Self {
        let roots = roots
            .into_iter()
            .filter_map(|(p, w)| Some((p.canonicalize().ok()?, w)))
            .collect();
        let data = data.canonicalize().unwrap_or(data);
        Sandbox { roots, data }
    }

    fn root_of(&self, p: &Path) -> Option<(PathBuf, bool)> {
        self.roots
            .iter()
            .filter(|(r, _)| p.starts_with(r))
            .max_by_key(|(r, _)| r.as_os_str().len())
            .cloned()
    }

    fn resolve(&self, raw: &str) -> Result<Resolved> {
        let p = Path::new(raw);
        if !p.is_absolute() || raw.contains('\0') {
            bail!("path must be absolute");
        }
        if p.components().any(|c| matches!(c, Component::ParentDir)) {
            bail!("path must not contain '..'");
        }
        // A root itself.
        if let Ok(c) = p.canonicalize() {
            if let Some((r, w)) = self.roots.iter().find(|(r, _)| *r == c) {
                return Ok(Resolved {
                    path: c,
                    root: r.clone(),
                    writable: *w,
                });
            }
        }
        let (Some(parent), Some(name)) = (p.parent(), p.file_name()) else {
            bail!("outside of the allowed roots");
        };
        let parent = parent
            .canonicalize()
            .with_context(|| format!("{}: no such directory", parent.display()))?;
        let path = parent.join(name);
        let Some((root, writable)) = self.root_of(&path) else {
            bail!("outside of the allowed roots");
        };
        let writable = writable && !path.starts_with(&self.data);
        Ok(Resolved {
            path,
            root,
            writable,
        })
    }

    /// For content access: the entry with symlinks followed must stay inside a root.
    fn resolve_content(&self, raw: &str) -> Result<Resolved> {
        let r = self.resolve(raw)?;
        if let Ok(target) = r.path.canonicalize() {
            let Some((_, w)) = self.root_of(&target) else {
                bail!("symlink points outside of the allowed roots");
            };
            if target.starts_with(&self.data) || !w {
                return Ok(Resolved {
                    writable: false,
                    ..r
                });
            }
        }
        Ok(r)
    }

    fn writable(&self, raw: &str) -> Result<Resolved> {
        let r = self.resolve_content(raw)?;
        if !r.writable {
            bail!("{} is read-only", r.path.display());
        }
        if r.path == r.root {
            bail!("cannot modify a root directory");
        }
        Ok(r)
    }

    pub fn list(&self, raw: &str) -> Result<FsListing> {
        let r = self.resolve_content(raw)?;
        let dir = r.path.canonicalize()?;
        let mut entries = Vec::new();
        let mut truncated = false;
        for e in fs::read_dir(&dir).with_context(|| format!("read {}", dir.display()))? {
            if entries.len() >= LIST_MAX {
                truncated = true;
                break;
            }
            let e = e?;
            if let Ok(entry) = entry(&e.path(), r.writable) {
                entries.push(entry);
            }
        }
        entries.sort_by(|a, b| {
            (a.kind != "dir", a.name.to_lowercase()).cmp(&(b.kind != "dir", b.name.to_lowercase()))
        });
        Ok(FsListing {
            path: dir.to_string_lossy().into_owned(),
            root: r.root.to_string_lossy().into_owned(),
            writable: r.writable,
            entries,
            truncated,
        })
    }

    pub fn stat(&self, raw: &str) -> Result<FsEntry> {
        let r = self.resolve(raw)?;
        entry(&r.path, r.writable)
    }

    pub fn read(&self, raw: &str, offset: u64, len: u32) -> Result<FsChunk> {
        let r = self.resolve_content(raw)?;
        let mut f = File::open(&r.path).with_context(|| format!("open {}", r.path.display()))?;
        let size = f.metadata()?.len();
        if !f.metadata()?.is_file() {
            bail!("not a regular file");
        }
        f.seek(SeekFrom::Start(offset))?;
        let mut buf = Vec::with_capacity(len.min(CHUNK_MAX) as usize);
        f.take(len.min(CHUNK_MAX) as u64).read_to_end(&mut buf)?;
        Ok(FsChunk {
            eof: offset + buf.len() as u64 >= size,
            data_b64: B64.encode(&buf),
            size,
        })
    }

    pub fn write(
        &self,
        raw: &str,
        content_b64: &str,
        in_place: bool,
        expected_mtime: Option<i64>,
    ) -> Result<FsEntry> {
        let r = self.writable(raw)?;
        let data = B64.decode(content_b64).context("content is not base64")?;
        if data.len() > WRITE_MAX {
            bail!("file is too large for the editor");
        }
        // Write through a symlink into its (in-root) target.
        let path = r.path.canonicalize().unwrap_or(r.path.clone());
        match fs::metadata(&path) {
            Ok(md) => {
                if !md.is_file() {
                    bail!("not a regular file");
                }
                if let Some(exp) = expected_mtime {
                    if md.mtime() != exp {
                        bail!("file changed on disk since it was opened");
                    }
                }
                self.save_version(&path)?;
                if in_place {
                    // Keeps the inode: single-file bind mounts (Caddyfile) see the change.
                    let mut f = OpenOptions::new().write(true).truncate(true).open(&path)?;
                    f.write_all(&data)?;
                    f.sync_all()?;
                } else {
                    let tmp = sibling(&path, "styk-tmp");
                    write_new(&tmp, &data, md.mode(), md.uid(), md.gid())?;
                    fs::rename(&tmp, &path)?;
                }
            }
            Err(_) => {
                let pmd = fs::metadata(path.parent().context("no parent")?)?;
                write_new(&path, &data, 0o644, pmd.uid(), pmd.gid())?;
            }
        }
        entry(&path, true)
    }

    pub fn upload(&self, raw: &str, offset: u64, data_b64: &str, last: bool) -> Result<FsEntry> {
        let r = self.writable(raw)?;
        let data = B64.decode(data_b64).context("chunk is not base64")?;
        let tmp = sibling(&r.path, "styk-upload");
        let mut f = if offset == 0 {
            File::create(&tmp)?
        } else {
            let f = OpenOptions::new()
                .append(true)
                .open(&tmp)
                .context("upload not started")?;
            if f.metadata()?.len() != offset {
                bail!("upload offset mismatch");
            }
            f
        };
        f.write_all(&data)?;
        if !last {
            return entry(&tmp, true);
        }
        f.sync_all()?;
        drop(f);
        let pmd = fs::metadata(r.path.parent().context("no parent")?)?;
        match fs::symlink_metadata(&r.path) {
            Ok(md) if md.is_file() => {
                self.save_version(&r.path)?;
                fs::set_permissions(&tmp, fs::Permissions::from_mode(md.mode() & 0o7777))?;
                let _ = nix::unistd::chown(&tmp, Some(md.uid().into()), Some(md.gid().into()));
            }
            Ok(_) => bail!("target exists and is not a file"),
            Err(_) => {
                fs::set_permissions(&tmp, fs::Permissions::from_mode(0o644))?;
                let _ = nix::unistd::chown(&tmp, Some(pmd.uid().into()), Some(pmd.gid().into()));
            }
        }
        fs::rename(&tmp, &r.path)?;
        entry(&r.path, true)
    }

    pub fn mkdir(&self, raw: &str) -> Result<FsEntry> {
        let r = self.writable(raw)?;
        fs::create_dir(&r.path).with_context(|| format!("mkdir {}", r.path.display()))?;
        let pmd = fs::metadata(r.path.parent().context("no parent")?)?;
        let _ = nix::unistd::chown(&r.path, Some(pmd.uid().into()), Some(pmd.gid().into()));
        entry(&r.path, true)
    }

    pub fn rename(&self, from: &str, to: &str) -> Result<FsEntry> {
        let a = self.writable(from)?;
        let b = self.writable(to)?;
        if fs::symlink_metadata(&b.path).is_ok() {
            bail!("{} already exists", b.path.display());
        }
        if b.path.starts_with(&a.path) {
            bail!("cannot move a directory into itself");
        }
        fs::rename(&a.path, &b.path)?;
        entry(&b.path, true)
    }

    pub fn trash(&self, raw: &str) -> Result<String> {
        let r = self.writable(raw)?;
        fs::symlink_metadata(&r.path)
            .with_context(|| format!("{}: not found", r.path.display()))?;
        let trash = self.data.join("trash");
        fs::create_dir_all(&trash)?;
        self.purge_trash(&trash);
        let name = r.path.file_name().unwrap_or_default().to_string_lossy();
        let dest = trash.join(format!("{}-{name}", now_ms()));
        fs::rename(&r.path, &dest).context("move to trash (different filesystem?)")?;
        fs::write(
            dest.with_extension("origin"),
            r.path.to_string_lossy().as_bytes(),
        )?;
        Ok(dest.to_string_lossy().into_owned())
    }

    pub fn history(&self, raw: &str) -> Result<Vec<FsVersion>> {
        let r = self.resolve_content(raw)?;
        let path = r.path.canonicalize().unwrap_or(r.path);
        let dir = self.history_dir(&path);
        let mut v: Vec<FsVersion> = match fs::read_dir(&dir) {
            Ok(rd) => rd
                .flatten()
                .filter_map(|e| {
                    let ts = e.file_name().to_str()?.parse::<i64>().ok()? / 1000;
                    Some(FsVersion {
                        path: e.path().to_string_lossy().into_owned(),
                        ts,
                        size: e.metadata().ok()?.len(),
                    })
                })
                .collect(),
            Err(_) => Vec::new(),
        };
        v.sort_by(|a, b| b.path.cmp(&a.path));
        Ok(v)
    }

    fn history_dir(&self, path: &Path) -> PathBuf {
        let rel = path.strip_prefix("/").unwrap_or(path);
        self.data.join("history").join(rel)
    }

    fn save_version(&self, path: &Path) -> Result<()> {
        let md = fs::metadata(path)?;
        if md.len() > HISTORY_MAX_SIZE {
            return Ok(());
        }
        let dir = self.history_dir(path);
        fs::create_dir_all(&dir)?;
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
        // Microsecond names, bumped on collision, double as the version timestamp.
        let mut ts = now_ms() * 1000;
        while dir.join(ts.to_string()).exists() {
            ts += 1;
        }
        fs::copy(path, dir.join(ts.to_string()))?;
        let mut old: Vec<_> = fs::read_dir(&dir)?.flatten().map(|e| e.path()).collect();
        old.sort();
        while old.len() > HISTORY_KEEP {
            let _ = fs::remove_file(old.remove(0));
        }
        Ok(())
    }

    fn purge_trash(&self, trash: &Path) {
        let cutoff = now_ms() - TRASH_DAYS * 86_400_000;
        let Ok(rd) = fs::read_dir(trash) else { return };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let ts: i64 = name
                .split('-')
                .next()
                .and_then(|t| t.parse().ok())
                .unwrap_or(i64::MAX);
            if ts < cutoff {
                let p = e.path();
                let _ = if p.is_dir() {
                    fs::remove_dir_all(&p)
                } else {
                    fs::remove_file(&p)
                };
            }
        }
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn sibling(path: &Path, tag: &str) -> PathBuf {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    path.with_file_name(format!(".{name}.{tag}"))
}

fn write_new(path: &Path, data: &[u8], mode: u32, uid: u32, gid: u32) -> Result<()> {
    let mut f = File::create(path)?;
    f.write_all(data)?;
    f.sync_all()?;
    fs::set_permissions(path, fs::Permissions::from_mode(mode & 0o7777))?;
    let _ = nix::unistd::chown(path, Some(uid.into()), Some(gid.into()));
    Ok(())
}

fn entry(path: &Path, writable: bool) -> Result<FsEntry> {
    let md = fs::symlink_metadata(path)?;
    let ft = md.file_type();
    let kind = if ft.is_symlink() {
        "symlink"
    } else if ft.is_dir() {
        "dir"
    } else if ft.is_file() {
        "file"
    } else {
        "other"
    };
    Ok(FsEntry {
        name: path
            .file_name()
            .unwrap_or(path.as_os_str())
            .to_string_lossy()
            .into_owned(),
        path: path.to_string_lossy().into_owned(),
        kind: kind.into(),
        size: md.len(),
        mtime: md.mtime(),
        mode: mode_string(md.mode()),
        uid: md.uid(),
        gid: md.gid(),
        writable,
    })
}

fn mode_string(mode: u32) -> String {
    let mut s = String::with_capacity(9);
    for shift in [6, 3, 0] {
        let m = (mode >> shift) & 7;
        s.push(if m & 4 != 0 { 'r' } else { '-' });
        s.push(if m & 2 != 0 { 'w' } else { '-' });
        s.push(if m & 1 != 0 { 'x' } else { '-' });
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup(name: &str) -> (PathBuf, Sandbox) {
        let base = std::env::temp_dir().join(format!("styk-fs-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        for d in ["rw/app", "ro", "outside", "rw/styk-data"] {
            fs::create_dir_all(base.join(d)).unwrap();
        }
        fs::write(base.join("outside/secret"), "s").unwrap();
        fs::write(base.join("ro/dump"), "d").unwrap();
        std::os::unix::fs::symlink(base.join("outside/secret"), base.join("rw/app/link")).unwrap();
        let sb = Sandbox::new(
            vec![(base.join("rw"), true), (base.join("ro"), false)],
            base.join("rw/styk-data"),
        );
        (base.canonicalize().unwrap(), sb)
    }

    fn s(p: PathBuf) -> String {
        p.to_string_lossy().into_owned()
    }

    #[test]
    fn sandboxing() {
        let (base, sb) = setup("sandbox");
        assert!(sb.list(&s(base.join("rw"))).is_ok());
        assert!(sb.list(&s(base.join("outside"))).is_err());
        assert!(sb
            .read(&format!("{}/rw/../outside/secret", base.display()), 0, 10)
            .is_err());
        assert!(
            sb.read(&s(base.join("rw/app/link")), 0, 10).is_err(),
            "symlink escape"
        );
        assert!(
            sb.stat(&s(base.join("rw/app/link"))).is_ok(),
            "the link itself is visible"
        );
        assert!(
            sb.write(&s(base.join("ro/new")), "", false, None).is_err(),
            "read-only root"
        );
        assert!(sb.trash(&s(base.join("rw"))).is_err(), "root itself");
        assert!(
            sb.write(&s(base.join("rw/styk-data/x")), "", false, None)
                .is_err(),
            "data dir"
        );
        assert!(sb.list("relative").is_err());
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn write_history_trash() {
        let (base, sb) = setup("write");
        let f = s(base.join("rw/app/Caddyfile"));
        sb.write(&f, &B64.encode("one"), true, None).unwrap();
        let ino = fs::metadata(&f).unwrap().ino();
        let mt = fs::metadata(&f).unwrap().mtime();
        sb.write(&f, &B64.encode("two"), true, Some(mt)).unwrap();
        assert_eq!(
            fs::metadata(&f).unwrap().ino(),
            ino,
            "in-place keeps the inode"
        );
        assert!(
            sb.write(&f, &B64.encode("x"), true, Some(mt - 100))
                .is_err(),
            "conflict"
        );
        sb.write(&f, &B64.encode("three"), false, None).unwrap();
        assert_eq!(fs::read_to_string(&f).unwrap(), "three");
        let h = sb.history(&f).unwrap();
        assert_eq!(h.len(), 2);
        assert!(sb.read(&h[0].path, 0, 100).is_ok(), "history is readable");

        let chunk = sb.read(&f, 1, 3).unwrap();
        assert_eq!(B64.decode(chunk.data_b64).unwrap(), b"hre");
        assert!(!chunk.eof);

        let up = s(base.join("rw/app/big.bin"));
        sb.upload(&up, 0, &B64.encode("ab"), false).unwrap();
        assert!(sb.upload(&up, 5, &B64.encode("cd"), false).is_err());
        sb.upload(&up, 2, &B64.encode("cd"), true).unwrap();
        assert_eq!(fs::read_to_string(&up).unwrap(), "abcd");

        sb.mkdir(&s(base.join("rw/app/sub"))).unwrap();
        sb.rename(&up, &s(base.join("rw/app/sub/big.bin"))).unwrap();
        let t = sb.trash(&s(base.join("rw/app/sub"))).unwrap();
        assert!(Path::new(&t).join("big.bin").exists());
        assert!(!base.join("rw/app/sub").exists());

        let l = sb.list(&s(base.join("rw/app"))).unwrap();
        assert!(l.writable);
        assert_eq!(
            l.entries.iter().filter(|e| e.name == "Caddyfile").count(),
            1
        );
        let _ = fs::remove_dir_all(&base);
    }
}
