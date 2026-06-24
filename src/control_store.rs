use camino::{Utf8Path, Utf8PathBuf};

pub fn control_dir(main_worktree: &Utf8Path) -> Utf8PathBuf {
    main_worktree.join(".wk")
}
