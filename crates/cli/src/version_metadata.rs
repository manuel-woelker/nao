use crate::build;
use nao_base::result::NaoResult;
use nao_base::shared_string::SharedString;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VersionMetadata {
    pub(crate) last_commit_date: SharedString,
    pub(crate) short_commit_id: SharedString,
    pub(crate) has_uncommitted_changes: bool,
}

pub(crate) fn render_version(metadata: &VersionMetadata) -> String {
    let dev_suffix = if metadata.has_uncommitted_changes {
        "-dev"
    } else {
        ""
    };

    format!(
        "{}-{}-{}{}",
        env!("CARGO_PKG_VERSION"),
        metadata.last_commit_date.as_str(),
        metadata.short_commit_id.as_str(),
        dev_suffix
    )
}

pub(crate) fn load_version_metadata() -> NaoResult<VersionMetadata> {
    Ok(VersionMetadata {
        last_commit_date: SharedString::from(normalize_commit_date(build::COMMIT_DATE)),
        short_commit_id: SharedString::from(normalize_short_commit(build::SHORT_COMMIT)),
        has_uncommitted_changes: !build::GIT_CLEAN,
    })
}

pub(crate) fn normalize_commit_date(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.len() >= 10 {
        trimmed[..10].to_owned()
    } else if trimmed.is_empty() {
        "unknown".to_owned()
    } else {
        trimmed.to_owned()
    }
}

pub(crate) fn normalize_short_commit(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        "unknown".to_owned()
    } else {
        trimmed.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::VersionMetadata;
    use super::normalize_commit_date;
    use super::normalize_short_commit;
    use super::render_version;
    use nao_base::shared_string::SharedString;

    #[test]
    fn renders_version_without_dev_suffix_for_clean_worktree() {
        let rendered = render_version(&VersionMetadata {
            last_commit_date: SharedString::from("2026-03-21"),
            short_commit_id: SharedString::from("abc1234"),
            has_uncommitted_changes: false,
        });

        assert_eq!(
            rendered,
            format!("{}-2026-03-21-abc1234", env!("CARGO_PKG_VERSION"))
        );
    }

    #[test]
    fn renders_version_with_dev_suffix_for_dirty_worktree() {
        let rendered = render_version(&VersionMetadata {
            last_commit_date: SharedString::from("2026-03-21"),
            short_commit_id: SharedString::from("abc1234"),
            has_uncommitted_changes: true,
        });

        assert_eq!(
            rendered,
            format!("{}-2026-03-21-abc1234-dev", env!("CARGO_PKG_VERSION"))
        );
    }

    #[test]
    fn normalizes_shadow_commit_date_to_calendar_date() {
        assert_eq!(
            normalize_commit_date("2026-03-21 14:22:11 +00:00"),
            "2026-03-21"
        );
    }

    #[test]
    fn falls_back_to_unknown_when_shadow_commit_date_is_missing() {
        assert_eq!(normalize_commit_date(""), "unknown");
    }

    #[test]
    fn falls_back_to_unknown_when_shadow_short_commit_is_missing() {
        assert_eq!(normalize_short_commit(""), "unknown");
    }
}
