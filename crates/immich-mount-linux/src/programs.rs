//! Finding Immich Mount's own programs wherever its channel put them: beside the running program (the packages put
//! them all in `/usr/bin`), on PATH, or in the Homebrew prefix, which a desktop session's PATH may not include.

use std::path::{Path, PathBuf};

/// Homebrew's prefix on Linux when nothing says otherwise.
const HOMEBREW_DEFAULT: &str = "/home/linuxbrew/.linuxbrew";

/// The first `name` that exists: in the `bin` of the Homebrew prefix the running program came from, beside the running
/// program, on PATH, then in another Homebrew prefix (`$HOMEBREW_PREFIX`, or Homebrew's default). A prefix's `bin`
/// holds stable links, so a path found there outlives the next `brew upgrade`, where the folder beside a program
/// from Homebrew is one version's.
pub fn find(name: &str) -> Option<PathBuf> {
    let running = std::env::current_exe().ok();
    let own_prefix = running.as_deref().and_then(homebrew_prefix_of);
    let beside = running.as_ref().map(|program| program.with_file_name(name));
    let on_path: Vec<PathBuf> =
        std::env::var_os("PATH").map(|path| std::env::split_paths(&path).map(|directory| directory.join(name)).collect()).unwrap_or_default();
    let other_prefixes = [std::env::var_os("HOMEBREW_PREFIX").map(PathBuf::from), Some(PathBuf::from(HOMEBREW_DEFAULT))];
    let in_prefix = |prefix: PathBuf| prefix.join("bin").join(name);
    own_prefix
        .map(in_prefix)
        .into_iter()
        .chain(beside)
        .chain(on_path)
        .chain(other_prefixes.into_iter().flatten().map(in_prefix))
        .find(|path| path.is_file())
}

/// The Homebrew prefix of a program that runs from its Cellar or Caskroom.
fn homebrew_prefix_of(program: &Path) -> Option<PathBuf> {
    let text = program.to_str()?;
    let at = text.find("/Cellar/").or_else(|| text.find("/Caskroom/"))?;
    Some(PathBuf::from(&text[..at]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_kegs_prefix_is_read_from_its_path() {
        let formula = Path::new("/home/linuxbrew/.linuxbrew/Cellar/immich-mount-headless/1.2.0/bin/immich-mount");
        assert_eq!(homebrew_prefix_of(formula), Some(PathBuf::from("/home/linuxbrew/.linuxbrew")));
        let cask = Path::new("/opt/brew/Caskroom/immich-mount/1.2.0/immich-mount-gnome");
        assert_eq!(homebrew_prefix_of(cask), Some(PathBuf::from("/opt/brew")));
        assert_eq!(homebrew_prefix_of(Path::new("/usr/bin/immich-mount")), None);
    }
}
