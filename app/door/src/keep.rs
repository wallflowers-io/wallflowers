//! KEEPING A PERSON'S DEVICE STATE between sessions (D-34 (c), Door step 7).
//!
//! One file per person on this node, `<seals>/<sha256(pk)>`: the state `Node::seal_state`
//! seals under a key off the person's own seed, with the node, the device and a counter
//! in its associated data. The counter's compare-and-set is at the auth service; this
//! file is ciphertext on the VM's disk (Software Security, Q2). One writer: the process
//! holds an exclusive lock on the person's lock file for its life.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Background publishes are sealed at most this often; a client's write is sealed
/// before it is acknowledged (Software Security, Q6).
pub const EVERY: Duration = Duration::from_secs(30);

pub struct Keeper {
    path: PathBuf,
    _lock: std::fs::File,
    /// The counter of the last seal written to disk.
    pub sealed: u64,
    /// The counter the auth service has confirmed.
    pub held: u64,
    published: u64,
    at: Instant,
}

impl Keeper {
    /// The person's seal on this node, locked to this process. Refused when another
    /// process holds it: two writers of one person's state is the fork this prevents.
    pub fn claim(dir: &Path, pk: &[u8; 32]) -> Result<Self, String> {
        std::fs::create_dir_all(dir).map_err(|e| format!("the seals directory {}: {e}", dir.display()))?;
        let name = hex::encode(<sha2::Sha256 as sha2::Digest>::digest(pk));
        let path = dir.join(&name);
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(dir.join(format!("{name}.lock")))
            .map_err(|e| format!("the seal's lock: {e}"))?;
        // SAFETY: flock on a descriptor this process owns, held until the File drops.
        if unsafe { libc::flock(std::os::fd::AsRawFd::as_raw_fd(&lock), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err("another process holds this person's state on this Door".into());
        }
        Ok(Self { path, _lock: lock, sealed: 0, held: 0, published: 0, at: Instant::now() })
    }

    pub fn read(&self) -> Option<Vec<u8>> {
        std::fs::read(&self.path).ok()
    }

    /// A refused or stale seal goes AT ONCE, so the keys of the leaves it held exist
    /// nowhere (Software Security, D-34 (c) condition 1).
    pub fn delete(&self) {
        if let Err(e) = std::fs::remove_file(&self.path) {
            if e.kind() != std::io::ErrorKind::NotFound {
                eprintln!("door: the refused seal could not be deleted: {e}");
            }
        }
    }

    /// Seal the Node's state at the next counter and put it on disk durably: a temp file
    /// beside it, fsynced, renamed over, the directory fsynced.
    pub fn seal(&mut self, n: &pacific_core::Node, node: &str) -> Result<(), String> {
        let counter = self.sealed.max(self.held) + 1;
        let blob = n.seal_state(node, counter).map_err(|e| format!("the state would not seal: {e}"))?;
        let tmp = self.path.with_extension("next");
        let written = (|| {
            let mut f = std::fs::File::create(&tmp)?;
            f.write_all(&blob)?;
            f.sync_all()?;
            std::fs::rename(&tmp, &self.path)?;
            std::fs::File::open(self.path.parent().unwrap_or(Path::new(".")))?.sync_all()
        })();
        if let Err(e) = written {
            let _ = std::fs::remove_file(&tmp);
            return Err(format!("the seal could not be written: {e}"));
        }
        self.sealed = counter;
        self.published = n.published();
        self.at = Instant::now();
        Ok(())
    }

    /// Something was published since the last seal, and the last is `EVERY` old.
    pub fn due(&self, n: &pacific_core::Node) -> bool {
        n.published() != self.published && self.at.elapsed() >= EVERY
    }

    /// Something was published since the last seal: the sent ledger rides the seal, so a
    /// publish no seal holds makes the next open stale (NC-76).
    pub fn behind(&self, n: &pacific_core::Node) -> bool {
        n.published() != self.published
    }

    /// A seal the auth service has not confirmed yet.
    pub fn unconfirmed(&self) -> bool {
        self.sealed > self.held
    }
}

/// THE CLAIM ENDS WITH ITS HOLDER, released outright (NC-98). flock's lock belongs to the open
/// file description, and a child forked in this process shares that description until it
/// execs: O_CLOEXEC closes its copy only then, and no flag on macOS or Linux keeps a
/// descriptor from a fork. Closing ours alone would leave the lock held for as long as such a
/// child lingers; LOCK_UN releases it on the description itself, whoever else holds a copy.
impl Drop for Keeper {
    fn drop(&mut self) {
        // SAFETY: flock on the descriptor this Keeper owns, still open until the File drops.
        unsafe { libc::flock(std::os::fd::AsRawFd::as_raw_fd(&self._lock), libc::LOCK_UN) };
    }
}

#[cfg(test)]
mod tests {
    use super::Keeper;

    /// One writer of a person's state on this node: a second claim is refused while the
    /// first holds it, another person's is not, and the claim ends with its holder.
    #[test]
    fn one_process_holds_a_persons_seal() {
        let dir = std::env::temp_dir().join(format!("door-keep-{}", std::process::id()));
        let first = Keeper::claim(&dir, &[1; 32]).expect("the first claim");
        assert!(Keeper::claim(&dir, &[1; 32]).is_err(), "a second writer of one person is refused");
        assert!(Keeper::claim(&dir, &[2; 32]).is_ok(), "another person's seal is its own");
        assert!(first.read().is_none() && !first.unconfirmed());
        drop(first);
        assert!(Keeper::claim(&dir, &[1; 32]).is_ok(), "the claim ends with its holder");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// NC-98: a child forked while the claim is held holds a copy of its descriptor, and so
    /// shares its lock, until it execs (O_CLOEXEC closes it only then). Any test of this binary
    /// spawning a process opens that window. The claim must still end with its holder, at once.
    #[test]
    fn the_claim_ends_with_its_holder_while_a_child_is_between_fork_and_exec() {
        use std::os::unix::process::CommandExt;
        let dir = std::env::temp_dir().join(format!("door-keep-fork-{}", std::process::id()));
        let first = Keeper::claim(&dir, &[3; 32]).expect("the first claim");
        // The child says it has forked (so holds its copy of every descriptor), then waits
        // before it execs: the window, held open.
        let mut pipe = [0; 2];
        assert_eq!(unsafe { libc::pipe(pipe.as_mut_ptr()) }, 0);
        let (read, write) = (pipe[0], pipe[1]);
        let spawner = std::thread::spawn(move || {
            let mut cmd = std::process::Command::new("true");
            // SAFETY: write and nanosleep are async-signal-safe.
            unsafe {
                cmd.pre_exec(move || {
                    libc::write(write, [1u8].as_ptr().cast(), 1);
                    let t = libc::timespec { tv_sec: 1, tv_nsec: 0 };
                    libc::nanosleep(&t, std::ptr::null_mut());
                    Ok(())
                });
            }
            cmd.status().expect("the child")
        });
        let mut forked = [0u8; 1];
        assert_eq!(unsafe { libc::read(read, forked.as_mut_ptr().cast(), 1) }, 1, "the child forked");
        drop(first);
        let again = Keeper::claim(&dir, &[3; 32]);
        assert!(spawner.join().unwrap().success());
        unsafe {
            libc::close(read);
            libc::close(write);
        }
        let _ = std::fs::remove_dir_all(&dir);
        assert!(again.is_ok(), "the claim ends with its holder, a child between fork and exec or not: {:?}", again.err());
    }
}
