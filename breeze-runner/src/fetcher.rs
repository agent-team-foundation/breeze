use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::bus::{Bus, Event};
use crate::config::RepoFilter;
use crate::gh_executor::{GhBucket, GhCommandSpec, GhExecutor};
use crate::json::Json;
use crate::util::{
    AppResult, app_error, current_epoch_secs, ensure_dir, parse_tsv_line, read_text_if_exists,
    write_text,
};

const NOTIFICATIONS_JQ: &str = ".[] | select(.subject.type != \"CheckSuite\" and .subject.type != \"Commit\") | [(.id // \"\"), (.subject.type // \"\"), (.reason // \"\"), (.repository.full_name // \"\"), (.subject.title // \"\"), (.subject.url // \"\"), ((.subject.latest_comment_url // .subject.url) // \"\"), (.updated_at // \"\"), (if .unread then \"1\" else \"0\" end)] | @tsv";

const LABEL_BATCH_SIZE: usize = 30;

/// A single notification prepared for the inbox.json contract consumed by the
/// statusline and the `/breeze` skill.
#[derive(Clone, Debug)]
pub struct InboxEntry {
    pub id: String,
    pub subject_type: String,
    pub reason: String,
    pub repo: String,
    pub title: String,
    pub url: String,
    pub last_actor: String,
    pub updated_at: String,
    pub unread: bool,
    pub priority: i64,
    pub number: Option<i64>,
    pub html_url: String,
    pub gh_state: Option<String>,
    pub labels: Vec<String>,
    pub breeze_status: String,
}

#[derive(Clone, Debug, Default)]
pub struct PollOutcome {
    pub total: usize,
    pub new_count: usize,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct Fetcher {
    executor: GhExecutor,
    host: String,
    inbox_dir: PathBuf,
    repo_filter: RepoFilter,
    bus: Option<Bus>,
}

impl Fetcher {
    pub fn new(
        executor: GhExecutor,
        host: String,
        inbox_dir: PathBuf,
        repo_filter: RepoFilter,
    ) -> Self {
        Self {
            executor,
            host,
            inbox_dir,
            repo_filter,
            bus: None,
        }
    }

    pub fn with_bus(mut self, bus: Bus) -> Self {
        self.bus = Some(bus);
        self
    }

    pub fn inbox_path(&self) -> PathBuf {
        self.inbox_dir.join("inbox.json")
    }

    pub fn activity_log_path(&self) -> PathBuf {
        self.inbox_dir.join("activity.log")
    }

    pub fn poll_once(&self) -> AppResult<PollOutcome> {
        ensure_dir(&self.inbox_dir)?;
        let mut warnings = Vec::new();

        let raw = match self.fetch_notifications() {
            Ok(value) => value,
            Err(error) => {
                warnings.push(format!("GitHub notifications fetch failed: {error}"));
                return Ok(PollOutcome {
                    total: 0,
                    new_count: 0,
                    warnings,
                });
            }
        };
        let mut entries = parse_notification_rows(&raw, &self.host, &self.repo_filter);
        sort_entries(&mut entries);

        match self.enrich_with_labels(&mut entries) {
            Ok(()) => {}
            Err(error) => warnings.push(format!("label enrichment degraded: {error}")),
        }
        for entry in entries.iter_mut() {
            entry.breeze_status =
                compute_breeze_status(entry.gh_state.as_deref(), &entry.labels).to_string();
        }

        let old_state = read_previous_inbox_state(&self.inbox_path());

        let poll_ts = format_utc_iso(current_epoch_secs());
        let new_events = diff_and_log(&old_state, &entries, &poll_ts);
        write_inbox(&self.inbox_path(), &entries, &poll_ts)?;
        if !new_events.is_empty() {
            append_activity_events(&self.activity_log_path(), &new_events)?;
        }
        let new_count = entries
            .iter()
            .filter(|entry| entry.breeze_status == "new")
            .count();
        if let Some(bus) = &self.bus {
            bus.publish(Event::InboxUpdated {
                last_poll: poll_ts.clone(),
                total: entries.len(),
                new_count,
            });
            for event in &new_events {
                bus.publish(Event::Activity(event.to_json_line()));
            }
        }
        Ok(PollOutcome {
            total: entries.len(),
            new_count,
            warnings,
        })
    }

    fn fetch_notifications(&self) -> AppResult<String> {
        self.executor.run_checked(&GhCommandSpec {
            context: "fetch notifications".to_string(),
            cwd: None,
            envs: self.host_env(),
            args: vec![
                "api".to_string(),
                "/notifications?all=true".to_string(),
                "--paginate".to_string(),
                "-H".to_string(),
                "X-GitHub-Api-Version: 2022-11-28".to_string(),
                "--jq".to_string(),
                NOTIFICATIONS_JQ.to_string(),
            ],
            bucket: GhBucket::Core,
            mutating: false,
        })
    }

    fn enrich_with_labels(&self, entries: &mut [InboxEntry]) -> AppResult<()> {
        let mut by_repo: BTreeMap<String, Vec<(i64, bool)>> = BTreeMap::new();
        for entry in entries.iter() {
            let Some(number) = entry.number else {
                continue;
            };
            let is_pr = entry.subject_type == "PullRequest";
            let is_issue = entry.subject_type == "Issue";
            if !(is_pr || is_issue) {
                continue;
            }
            by_repo
                .entry(entry.repo.clone())
                .or_default()
                .push((number, is_pr));
        }

        let mut info: HashMap<(String, i64), LabelInfo> = HashMap::new();
        for (repo, mut items) in by_repo {
            items.sort_by_key(|(number, _)| *number);
            items.dedup();
            for batch in items.chunks(LABEL_BATCH_SIZE) {
                match self.fetch_label_batch(&repo, batch) {
                    Ok(rows) => {
                        for row in rows {
                            info.insert((repo.clone(), row.number), row.into_info());
                        }
                    }
                    Err(error) => {
                        return Err(app_error(format!(
                            "GraphQL label enrichment for {repo} failed: {error}"
                        )));
                    }
                }
            }
        }

        for entry in entries.iter_mut() {
            let Some(number) = entry.number else {
                continue;
            };
            if let Some(row) = info.get(&(entry.repo.clone(), number)) {
                entry.gh_state = row.gh_state.clone();
                entry.labels = row.labels.clone();
            }
        }
        Ok(())
    }

    fn fetch_label_batch(
        &self,
        repo: &str,
        batch: &[(i64, bool)],
    ) -> AppResult<Vec<LabelRow>> {
        let (owner, name) = split_repo(repo)?;
        let query = build_label_query(&owner, &name, batch);
        let jq = ".data.repository // {} | to_entries | .[] | [(.value.number | tostring), (.value.state // \"\"), (.value.labels.nodes | map(.name) | join(\",\"))] | @tsv";
        let stdout = self.executor.run_checked(&GhCommandSpec {
            context: format!("graphql labels for {repo}"),
            cwd: None,
            envs: self.host_env(),
            args: vec![
                "api".to_string(),
                "graphql".to_string(),
                "-f".to_string(),
                format!("query={query}"),
                "--jq".to_string(),
                jq.to_string(),
            ],
            bucket: GhBucket::Core,
            mutating: false,
        })?;
        let mut rows = Vec::new();
        for line in stdout.lines().filter(|line| !line.trim().is_empty()) {
            let fields = parse_tsv_line(line);
            if fields.len() < 3 {
                continue;
            }
            let Ok(number) = fields[0].parse::<i64>() else {
                continue;
            };
            let state = fields[1].clone();
            let labels = fields[2]
                .split(',')
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .collect::<Vec<_>>();
            rows.push(LabelRow {
                number,
                state: if state.is_empty() { None } else { Some(state) },
                labels,
            });
        }
        Ok(rows)
    }

    fn host_env(&self) -> Vec<(String, String)> {
        vec![("GH_HOST".to_string(), self.host.clone())]
    }
}

#[derive(Clone, Debug)]
struct LabelRow {
    number: i64,
    state: Option<String>,
    labels: Vec<String>,
}

impl LabelRow {
    fn into_info(self) -> LabelInfo {
        LabelInfo {
            gh_state: self.state,
            labels: self.labels,
        }
    }
}

#[derive(Clone, Debug)]
struct LabelInfo {
    gh_state: Option<String>,
    labels: Vec<String>,
}

fn split_repo(repo: &str) -> AppResult<(String, String)> {
    let (owner, name) = repo
        .split_once('/')
        .ok_or_else(|| app_error(format!("invalid repo identifier `{repo}`")))?;
    Ok((owner.to_string(), name.to_string()))
}

fn build_label_query(owner: &str, name: &str, batch: &[(i64, bool)]) -> String {
    let mut fragments = String::new();
    for (number, is_pr) in batch {
        let kind = if *is_pr { "pullRequest" } else { "issue" };
        fragments.push_str(&format!(
            " n{number}: {kind}(number: {number}) {{ number state labels(first: 10) {{ nodes {{ name }} }} }}"
        ));
    }
    format!("query {{ repository(owner: \"{owner}\", name: \"{name}\") {{{fragments} }} }}")
}

pub fn parse_notification_rows(
    raw: &str,
    host: &str,
    repo_filter: &RepoFilter,
) -> Vec<InboxEntry> {
    let mut entries = Vec::new();
    for line in raw.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let fields = parse_tsv_line(line);
        if fields.len() < 9 {
            continue;
        }
        let repo = fields[3].clone();
        if repo.is_empty() || !repo_filter.matches_repo(&repo) {
            continue;
        }
        let subject_type = fields[1].clone();
        let reason = fields[2].clone();
        let id = fields[0].clone();
        let title = fields[4].clone();
        let url = fields[5].clone();
        let last_actor = fields[6].clone();
        let updated_at = fields[7].clone();
        let unread = fields[8] == "1";

        let number = extract_trailing_number(&url);
        let html_url = html_url_for(host, &repo, &subject_type, number);
        let priority = priority_for_reason(&reason);

        entries.push(InboxEntry {
            id,
            subject_type,
            reason,
            repo,
            title,
            url,
            last_actor,
            updated_at,
            unread,
            priority,
            number,
            html_url,
            gh_state: None,
            labels: Vec::new(),
            breeze_status: "new".to_string(),
        });
    }
    entries
}

pub fn compute_breeze_status(gh_state: Option<&str>, labels: &[String]) -> &'static str {
    let has = |needle: &str| labels.iter().any(|label| label == needle);
    if has("breeze:done") {
        return "done";
    }
    if matches!(gh_state, Some("MERGED") | Some("CLOSED")) {
        return "done";
    }
    if has("breeze:human") {
        return "human";
    }
    if has("breeze:wip") {
        return "wip";
    }
    "new"
}

fn priority_for_reason(reason: &str) -> i64 {
    match reason {
        "review_requested" => 1,
        "mention" => 2,
        "assign" => 3,
        "participating" => 4,
        _ => 5,
    }
}

fn extract_trailing_number(url: &str) -> Option<i64> {
    if !(url.contains("/pulls/") || url.contains("/issues/")) {
        return None;
    }
    let segment = url.rsplit('/').next()?;
    let digits: String = segment
        .chars()
        .take_while(|character| character.is_ascii_digit())
        .collect();
    digits.parse::<i64>().ok()
}

fn html_url_for(host: &str, repo: &str, subject_type: &str, number: Option<i64>) -> String {
    let base = format!("https://{host}/{repo}");
    match (subject_type, number) {
        ("PullRequest", Some(number)) => format!("{base}/pull/{number}"),
        ("Issue", Some(number)) => format!("{base}/issues/{number}"),
        _ => base,
    }
}

fn sort_entries(entries: &mut [InboxEntry]) {
    entries.sort_by(|left, right| {
        left.priority
            .cmp(&right.priority)
            .then_with(|| right.updated_at.cmp(&left.updated_at))
            .then_with(|| left.id.cmp(&right.id))
    });
}

#[derive(Clone, Debug, Default)]
pub struct PreviousInboxState {
    pub statuses: HashMap<String, String>,
    pub ids: Vec<String>,
    pub metadata: HashMap<String, (String, String, String, String)>,
}

pub fn read_previous_inbox_state(path: &Path) -> PreviousInboxState {
    let Ok(Some(contents)) = read_text_if_exists(path) else {
        return PreviousInboxState::default();
    };
    read_inbox_state_from_jq(&contents).unwrap_or_default()
}

fn read_inbox_state_from_jq(inbox_contents: &str) -> AppResult<PreviousInboxState> {
    let mut command = Command::new("jq");
    command.arg("-r").arg(
        ".notifications[] | [(.id // \"\"), (.breeze_status // \"new\"), (.type // \"\"), (.repo // \"\"), (.title // \"\"), (.html_url // \"\")] | @tsv",
    );
    command.stdin(std::process::Stdio::piped());
    command.stdout(std::process::Stdio::piped());
    command.stderr(std::process::Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|error| app_error(format!("failed to spawn jq: {error}")))?;
    use std::io::Write;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(inbox_contents.as_bytes())
            .map_err(|error| app_error(format!("failed to write inbox to jq: {error}")))?;
    }
    let output = child
        .wait_with_output()
        .map_err(|error| app_error(format!("jq wait failed: {error}")))?;
    if !output.status.success() {
        return Ok(PreviousInboxState::default());
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut state = PreviousInboxState::default();
    for line in stdout.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let fields = parse_tsv_line(line);
        if fields.len() < 6 {
            continue;
        }
        let id = fields[0].clone();
        let status = fields[1].clone();
        let subject_type = fields[2].clone();
        let repo = fields[3].clone();
        let title = fields[4].clone();
        let html_url = fields[5].clone();
        state.ids.push(id.clone());
        state.statuses.insert(id.clone(), status);
        state
            .metadata
            .insert(id, (subject_type, repo, title, html_url));
    }
    Ok(state)
}

#[derive(Clone, Debug)]
pub enum ActivityEvent {
    New {
        id: String,
        subject_type: String,
        repo: String,
        title: String,
        html_url: String,
        ts: String,
    },
    Transition {
        id: String,
        subject_type: String,
        repo: String,
        title: String,
        html_url: String,
        from: String,
        to: String,
        ts: String,
    },
}

impl ActivityEvent {
    fn to_json_line(&self) -> String {
        match self {
            ActivityEvent::New {
                id,
                subject_type,
                repo,
                title,
                html_url,
                ts,
            } => Json::Object(vec![
                ("ts".to_string(), Json::str(ts.clone())),
                ("event".to_string(), Json::str("new")),
                ("id".to_string(), Json::str(id.clone())),
                ("type".to_string(), Json::str(subject_type.clone())),
                ("repo".to_string(), Json::str(repo.clone())),
                ("title".to_string(), Json::str(title.clone())),
                ("url".to_string(), Json::str(html_url.clone())),
            ])
            .encode(),
            ActivityEvent::Transition {
                id,
                subject_type,
                repo,
                title,
                html_url,
                from,
                to,
                ts,
            } => Json::Object(vec![
                ("ts".to_string(), Json::str(ts.clone())),
                ("event".to_string(), Json::str("transition")),
                ("id".to_string(), Json::str(id.clone())),
                ("type".to_string(), Json::str(subject_type.clone())),
                ("repo".to_string(), Json::str(repo.clone())),
                ("title".to_string(), Json::str(title.clone())),
                ("url".to_string(), Json::str(html_url.clone())),
                ("from".to_string(), Json::str(from.clone())),
                ("to".to_string(), Json::str(to.clone())),
            ])
            .encode(),
        }
    }
}

pub fn diff_and_log(
    old: &PreviousInboxState,
    entries: &[InboxEntry],
    poll_ts: &str,
) -> Vec<ActivityEvent> {
    let mut events = Vec::new();
    for entry in entries {
        let seen_before = old.statuses.contains_key(&entry.id);
        if !seen_before {
            events.push(ActivityEvent::New {
                id: entry.id.clone(),
                subject_type: entry.subject_type.clone(),
                repo: entry.repo.clone(),
                title: entry.title.clone(),
                html_url: entry.html_url.clone(),
                ts: poll_ts.to_string(),
            });
            continue;
        }
        let Some(previous) = old.statuses.get(&entry.id) else {
            continue;
        };
        if previous == &entry.breeze_status {
            continue;
        }
        // Ignore new→done transitions driven purely by GitHub state so the
        // activity log focuses on human-observable label changes.
        if previous == "new" && entry.breeze_status == "done" {
            continue;
        }
        events.push(ActivityEvent::Transition {
            id: entry.id.clone(),
            subject_type: entry.subject_type.clone(),
            repo: entry.repo.clone(),
            title: entry.title.clone(),
            html_url: entry.html_url.clone(),
            from: previous.clone(),
            to: entry.breeze_status.clone(),
            ts: poll_ts.to_string(),
        });
    }
    events
}

pub fn write_inbox(path: &Path, entries: &[InboxEntry], last_poll: &str) -> AppResult<()> {
    if let Some(parent) = path.parent() {
        ensure_dir(parent)?;
    }
    let payload = Json::Object(vec![
        ("last_poll".to_string(), Json::str(last_poll)),
        (
            "notifications".to_string(),
            Json::Array(entries.iter().map(entry_to_json).collect()),
        ),
    ]);
    let tmp_path = path.with_extension("json.tmp");
    write_text(&tmp_path, &payload.encode())?;
    fs::rename(&tmp_path, path)
        .map_err(|error| app_error(format!("atomic inbox rename failed: {error}")))?;
    Ok(())
}

fn entry_to_json(entry: &InboxEntry) -> Json {
    Json::Object(vec![
        ("id".to_string(), Json::str(entry.id.clone())),
        ("type".to_string(), Json::str(entry.subject_type.clone())),
        ("reason".to_string(), Json::str(entry.reason.clone())),
        ("repo".to_string(), Json::str(entry.repo.clone())),
        ("title".to_string(), Json::str(entry.title.clone())),
        ("url".to_string(), Json::str(entry.url.clone())),
        ("last_actor".to_string(), Json::str(entry.last_actor.clone())),
        (
            "updated_at".to_string(),
            Json::str(entry.updated_at.clone()),
        ),
        ("unread".to_string(), Json::Bool(entry.unread)),
        ("priority".to_string(), Json::Number(entry.priority)),
        ("number".to_string(), Json::number_or_null(entry.number)),
        ("html_url".to_string(), Json::str(entry.html_url.clone())),
        (
            "gh_state".to_string(),
            Json::str_or_null(entry.gh_state.clone()),
        ),
        (
            "labels".to_string(),
            Json::array_of_strings(entry.labels.clone()),
        ),
        (
            "breeze_status".to_string(),
            Json::str(entry.breeze_status.clone()),
        ),
    ])
}

pub fn append_activity_events(path: &Path, events: &[ActivityEvent]) -> AppResult<()> {
    if events.is_empty() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        ensure_dir(parent)?;
    }
    let existing = read_text_if_exists(path)?.unwrap_or_default();
    let mut buffer = existing;
    if !buffer.is_empty() && !buffer.ends_with('\n') {
        buffer.push('\n');
    }
    for event in events {
        buffer.push_str(&event.to_json_line());
        buffer.push('\n');
    }
    write_text(path, &buffer)
}

pub fn resolve_inbox_dir() -> AppResult<PathBuf> {
    if let Some(explicit) = std::env::var_os("BREEZE_DIR") {
        return Ok(PathBuf::from(explicit));
    }
    Ok(crate::util::home_dir()?.join(".breeze"))
}

pub fn cleanup_expired_claims(claims_dir: &Path, timeout_secs: u64) -> AppResult<()> {
    if !claims_dir.exists() {
        return Ok(());
    }
    let now = current_epoch_secs();
    let entries = match fs::read_dir(claims_dir) {
        Ok(value) => value,
        Err(_) => return Ok(()),
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let marker = path.join("claimed_at");
        let Ok(Some(contents)) = read_text_if_exists(&marker) else {
            continue;
        };
        let trimmed = contents.trim();
        let Some(claimed_epoch) =
            crate::util::parse_github_timestamp_epoch(trimmed)
        else {
            continue;
        };
        if now.saturating_sub(claimed_epoch) >= timeout_secs {
            let _ = fs::remove_dir_all(&path);
        }
    }
    Ok(())
}

fn format_utc_iso(epoch_seconds: u64) -> String {
    // Hand-rolled ISO-8601 formatter since we stay zero-dep.
    let days = (epoch_seconds / 86_400) as i64;
    let seconds_in_day = (epoch_seconds % 86_400) as u32;
    let hour = seconds_in_day / 3_600;
    let minute = (seconds_in_day % 3_600) / 60;
    let second = seconds_in_day % 60;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z",
        year = year,
        month = month,
        day = day,
        hour = hour,
        minute = minute,
        second = second
    )
}

fn civil_from_days(mut days: i64) -> (i32, u32, u32) {
    // Howard Hinnant's algorithm, inverse of days_from_civil in util.rs.
    days += 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let day_of_era = (days - era * 146_097) as u64;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era as i64 + era * 400;
    let day_of_year = day_of_era
        - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    };
    let year = if month <= 2 { year + 1 } else { year };
    (year as i32, month as u32, day as u32)
}

#[cfg(test)]
mod tests {
    use super::{
        InboxEntry, PreviousInboxState, build_label_query, civil_from_days, compute_breeze_status,
        diff_and_log, entry_to_json, extract_trailing_number, format_utc_iso, html_url_for,
        parse_notification_rows, sort_entries,
    };
    use crate::config::RepoFilter;
    use crate::json::Json;
    use std::collections::HashMap;

    #[test]
    fn parses_tab_separated_notification_rows() {
        let raw = "123\tPullRequest\tmention\towner/repo\tFix things\thttps://api.github.com/repos/owner/repo/pulls/45\thttps://api.github.com/repos/owner/repo/issues/comments/99\t2026-04-14T12:00:00Z\t1";
        let entries = parse_notification_rows(raw, "github.com", &RepoFilter::default());
        assert_eq!(entries.len(), 1);
        let entry = &entries[0];
        assert_eq!(entry.id, "123");
        assert_eq!(entry.subject_type, "PullRequest");
        assert_eq!(entry.reason, "mention");
        assert_eq!(entry.repo, "owner/repo");
        assert_eq!(entry.number, Some(45));
        assert_eq!(entry.html_url, "https://github.com/owner/repo/pull/45");
        assert!(entry.unread);
        assert_eq!(entry.priority, 2);
    }

    #[test]
    fn drops_rows_outside_repo_allowlist() {
        let filter = RepoFilter::parse_csv("allowed/repo").expect("filter");
        let raw = "1\tPullRequest\tmention\tother/repo\tT\thttps://api.github.com/repos/other/repo/pulls/1\t\t2026-04-14T00:00:00Z\t1\n2\tPullRequest\tmention\tallowed/repo\tT\thttps://api.github.com/repos/allowed/repo/pulls/2\t\t2026-04-14T00:00:00Z\t1";
        let entries = parse_notification_rows(raw, "github.com", &filter);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].repo, "allowed/repo");
    }

    #[test]
    fn html_url_handles_prs_issues_and_fallback() {
        assert_eq!(
            html_url_for("github.com", "o/r", "PullRequest", Some(1)),
            "https://github.com/o/r/pull/1"
        );
        assert_eq!(
            html_url_for("github.com", "o/r", "Issue", Some(7)),
            "https://github.com/o/r/issues/7"
        );
        assert_eq!(
            html_url_for("github.com", "o/r", "Discussion", None),
            "https://github.com/o/r"
        );
    }

    #[test]
    fn extract_trailing_number_parses_pulls_and_issues() {
        assert_eq!(
            extract_trailing_number("https://api.github.com/repos/o/r/pulls/42"),
            Some(42)
        );
        assert_eq!(
            extract_trailing_number("https://api.github.com/repos/o/r/issues/99"),
            Some(99)
        );
        assert_eq!(extract_trailing_number("https://github.com/o/r"), None);
    }

    #[test]
    fn breeze_status_precedence_follows_shell_logic() {
        assert_eq!(
            compute_breeze_status(None, &["breeze:done".to_string()]),
            "done"
        );
        assert_eq!(compute_breeze_status(Some("MERGED"), &[]), "done");
        assert_eq!(compute_breeze_status(Some("CLOSED"), &[]), "done");
        assert_eq!(
            compute_breeze_status(Some("OPEN"), &["breeze:human".to_string()]),
            "human"
        );
        assert_eq!(
            compute_breeze_status(Some("OPEN"), &["breeze:wip".to_string()]),
            "wip"
        );
        assert_eq!(compute_breeze_status(Some("OPEN"), &[]), "new");
    }

    #[test]
    fn breeze_status_done_beats_human_and_wip() {
        let labels = [
            "breeze:done".to_string(),
            "breeze:human".to_string(),
            "breeze:wip".to_string(),
        ];
        assert_eq!(compute_breeze_status(Some("OPEN"), &labels), "done");
    }

    #[test]
    fn label_query_contains_aliases_per_item() {
        let query = build_label_query("o", "r", &[(1, true), (2, false)]);
        assert!(query.contains("n1: pullRequest(number: 1)"));
        assert!(query.contains("n2: issue(number: 2)"));
        assert!(query.starts_with("query { repository(owner: \"o\", name: \"r\")"));
    }

    #[test]
    fn diff_detects_new_and_transition_events_and_ignores_new_to_done() {
        let entry_new = InboxEntry {
            id: "id-new".to_string(),
            subject_type: "PullRequest".to_string(),
            reason: "mention".to_string(),
            repo: "o/r".to_string(),
            title: "new pr".to_string(),
            url: "https://api.github.com/repos/o/r/pulls/1".to_string(),
            last_actor: "".to_string(),
            updated_at: "2026-04-14T00:00:00Z".to_string(),
            unread: true,
            priority: 2,
            number: Some(1),
            html_url: "https://github.com/o/r/pull/1".to_string(),
            gh_state: Some("OPEN".to_string()),
            labels: Vec::new(),
            breeze_status: "new".to_string(),
        };
        let entry_transition = InboxEntry {
            id: "id-transition".to_string(),
            subject_type: "Issue".to_string(),
            reason: "assign".to_string(),
            repo: "o/r".to_string(),
            title: "t".to_string(),
            url: String::new(),
            last_actor: String::new(),
            updated_at: "2026-04-14T00:00:00Z".to_string(),
            unread: false,
            priority: 3,
            number: Some(2),
            html_url: "https://github.com/o/r/issues/2".to_string(),
            gh_state: Some("OPEN".to_string()),
            labels: vec!["breeze:wip".to_string()],
            breeze_status: "wip".to_string(),
        };
        let entry_done_autopass = InboxEntry {
            id: "id-auto-done".to_string(),
            subject_type: "PullRequest".to_string(),
            reason: "mention".to_string(),
            repo: "o/r".to_string(),
            title: "auto-done".to_string(),
            url: String::new(),
            last_actor: String::new(),
            updated_at: "2026-04-14T00:00:00Z".to_string(),
            unread: false,
            priority: 2,
            number: Some(3),
            html_url: "https://github.com/o/r/pull/3".to_string(),
            gh_state: Some("MERGED".to_string()),
            labels: Vec::new(),
            breeze_status: "done".to_string(),
        };

        let mut previous = PreviousInboxState::default();
        previous.ids.push("id-transition".to_string());
        previous
            .statuses
            .insert("id-transition".to_string(), "new".to_string());
        previous.ids.push("id-auto-done".to_string());
        previous
            .statuses
            .insert("id-auto-done".to_string(), "new".to_string());

        let events = diff_and_log(
            &previous,
            &[entry_new, entry_transition, entry_done_autopass],
            "2026-04-14T01:00:00Z",
        );
        assert_eq!(events.len(), 2);
        let kinds = events
            .iter()
            .map(|event| match event {
                super::ActivityEvent::New { .. } => "new",
                super::ActivityEvent::Transition { .. } => "transition",
            })
            .collect::<Vec<_>>();
        assert_eq!(kinds, vec!["new", "transition"]);
    }

    #[test]
    fn sort_entries_orders_by_priority_then_recency() {
        let mut entries = vec![
            make_entry("a", 5, "2026-04-14T00:00:00Z"),
            make_entry("b", 1, "2026-04-13T00:00:00Z"),
            make_entry("c", 2, "2026-04-14T00:00:00Z"),
            make_entry("d", 2, "2026-04-15T00:00:00Z"),
        ];
        sort_entries(&mut entries);
        let order = entries.iter().map(|entry| entry.id.clone()).collect::<Vec<_>>();
        assert_eq!(order, vec!["b", "d", "c", "a"]);
    }

    fn make_entry(id: &str, priority: i64, updated_at: &str) -> InboxEntry {
        InboxEntry {
            id: id.to_string(),
            subject_type: "PullRequest".to_string(),
            reason: "mention".to_string(),
            repo: "o/r".to_string(),
            title: String::new(),
            url: String::new(),
            last_actor: String::new(),
            updated_at: updated_at.to_string(),
            unread: true,
            priority,
            number: Some(1),
            html_url: String::new(),
            gh_state: None,
            labels: Vec::new(),
            breeze_status: "new".to_string(),
        }
    }

    #[test]
    fn entry_to_json_emits_all_inbox_fields() {
        let entry = make_entry("42", 2, "2026-04-14T00:00:00Z");
        let value = entry_to_json(&entry);
        let rendered = value.encode();
        for needle in [
            "\"id\":\"42\"",
            "\"type\":\"PullRequest\"",
            "\"reason\":\"mention\"",
            "\"repo\":\"o/r\"",
            "\"priority\":2",
            "\"number\":1",
            "\"unread\":true",
            "\"labels\":[]",
            "\"breeze_status\":\"new\"",
        ] {
            assert!(rendered.contains(needle), "missing `{needle}` in `{rendered}`");
        }
        // Null fields render as literal null.
        assert!(rendered.contains("\"gh_state\":null"));
    }

    #[test]
    fn civil_from_days_and_back() {
        let epoch = super::format_utc_iso(0);
        assert_eq!(epoch, "1970-01-01T00:00:00Z");
        let mid = format_utc_iso(946_684_800);
        assert_eq!(mid, "2000-01-01T00:00:00Z");
        let today = format_utc_iso(1_717_200_000);
        assert!(today.starts_with("2024-06-01"));
        let (year, month, day) = civil_from_days(0);
        assert_eq!((year, month, day), (1970, 1, 1));
        // Silence unused warning when the helper binding is unused in a path.
        let _ = Json::Null;
        let _ = HashMap::<String, String>::new();
    }
}
