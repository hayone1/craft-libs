//! Host paths: names in a host's own storage.

/// Whether a file name is a path in a host's own storage, `<scheme>:<folder>/<path>` such as
/// `cloud:home/art/logo.svg`, rather than a bare file name. A one-letter scheme is a Windows
/// drive, not a host.
pub fn is_host_path(name: &str) -> bool {
    name.split_once(':').is_some_and(|(scheme, rest)| {
        rest.contains('/')
            && scheme.len() > 1
            && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
            && scheme.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-'))
    })
}

#[cfg(test)]
mod tests {
    use super::is_host_path;

    #[test]
    fn host_paths_have_a_scheme_and_a_folder() {
        assert!(is_host_path("cloud:home/art/photo.psd"));
        assert!(is_host_path("team-drive:shared/logo.svg"));
        assert!(is_host_path("s3+eu.west:bucket/a.png"));
    }

    #[test]
    fn bare_names_drives_and_odd_schemes_are_not_host_paths() {
        assert!(!is_host_path("photo.psd"));
        assert!(!is_host_path("C:/Users/me/photo.psd"));
        assert!(!is_host_path("c:\\Users\\me\\photo.psd"));
        assert!(!is_host_path("cloud:photo.psd"), "no folder");
        assert!(!is_host_path("1cloud:home/photo.psd"), "a scheme starts with a letter");
        assert!(!is_host_path("my cloud:home/photo.psd"));
        assert!(!is_host_path(":home/photo.psd"));
        assert!(!is_host_path(""));
    }
}
