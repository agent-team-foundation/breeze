#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TaskKind {
    ReviewRequest,
    Mention,
    Comment,
    AssignedIssue,
    AssignedPullRequest,
    Discussion,
    Other,
}

impl TaskKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            TaskKind::ReviewRequest => "review_request",
            TaskKind::Mention => "mention",
            TaskKind::Comment => "comment",
            TaskKind::AssignedIssue => "assigned_issue",
            TaskKind::AssignedPullRequest => "assigned_pull_request",
            TaskKind::Discussion => "discussion",
            TaskKind::Other => "other",
        }
    }

    pub fn from_str(value: &str) -> Option<Self> {
        Some(match value {
            "review_request" => TaskKind::ReviewRequest,
            "mention" => TaskKind::Mention,
            "comment" => TaskKind::Comment,
            "assigned_issue" => TaskKind::AssignedIssue,
            "assigned_pull_request" => TaskKind::AssignedPullRequest,
            "discussion" => TaskKind::Discussion,
            "other" => TaskKind::Other,
            _ => return None,
        })
    }
}

pub fn should_process_reason(reason: &str) -> bool {
    matches!(
        reason,
        "review_requested"
            | "comment"
            | "mention"
            | "team_mention"
            | "assign"
            | "author"
            | "manual"
    )
}

pub fn priority_for(kind: &TaskKind, reason: &str) -> u8 {
    match kind {
        TaskKind::ReviewRequest => 100,
        TaskKind::Mention => 95,
        TaskKind::Discussion => 90,
        TaskKind::Comment => 85,
        TaskKind::AssignedPullRequest => 80,
        TaskKind::AssignedIssue => 70,
        TaskKind::Other => {
            if reason == "review_requested" {
                100
            } else {
                50
            }
        }
    }
}

pub fn classify_notification(subject_type: &str, reason: &str) -> TaskKind {
    if reason == "review_requested" {
        return TaskKind::ReviewRequest;
    }
    if reason == "mention" || reason == "team_mention" {
        return TaskKind::Mention;
    }
    if subject_type.contains("Discussion") {
        return TaskKind::Discussion;
    }
    if reason == "comment" || reason == "author" || reason == "manual" {
        return TaskKind::Comment;
    }
    if reason == "assign" {
        if subject_type == "PullRequest" {
            return TaskKind::AssignedPullRequest;
        }
        return TaskKind::AssignedIssue;
    }
    TaskKind::Other
}

#[cfg(test)]
mod tests {
    use super::{TaskKind, classify_notification, priority_for, should_process_reason};

    #[test]
    fn classifies_review_request_ahead_of_subject_type() {
        assert_eq!(
            classify_notification("PullRequest", "review_requested"),
            TaskKind::ReviewRequest
        );
    }

    #[test]
    fn classifies_mentions_as_mention_kind() {
        assert_eq!(
            classify_notification("Issue", "mention"),
            TaskKind::Mention
        );
        assert_eq!(
            classify_notification("PullRequest", "team_mention"),
            TaskKind::Mention
        );
    }

    #[test]
    fn classifies_assign_by_subject_type() {
        assert_eq!(
            classify_notification("PullRequest", "assign"),
            TaskKind::AssignedPullRequest
        );
        assert_eq!(
            classify_notification("Issue", "assign"),
            TaskKind::AssignedIssue
        );
    }

    #[test]
    fn classifies_discussions_when_subject_type_mentions_discussion() {
        assert_eq!(
            classify_notification("Discussion", "subscribed"),
            TaskKind::Discussion
        );
    }

    #[test]
    fn classifies_comment_for_author_and_manual_reasons() {
        assert_eq!(
            classify_notification("Issue", "author"),
            TaskKind::Comment
        );
        assert_eq!(
            classify_notification("PullRequest", "manual"),
            TaskKind::Comment
        );
    }

    #[test]
    fn returns_other_for_unknown_reason() {
        assert_eq!(
            classify_notification("Commit", "ci_activity"),
            TaskKind::Other
        );
    }

    #[test]
    fn priority_ranks_review_highest_and_assigned_issue_lowest() {
        assert_eq!(priority_for(&TaskKind::ReviewRequest, "review_requested"), 100);
        assert_eq!(priority_for(&TaskKind::Mention, "mention"), 95);
        assert_eq!(priority_for(&TaskKind::Discussion, "subscribed"), 90);
        assert_eq!(priority_for(&TaskKind::Comment, "comment"), 85);
        assert_eq!(priority_for(&TaskKind::AssignedPullRequest, "assign"), 80);
        assert_eq!(priority_for(&TaskKind::AssignedIssue, "assign"), 70);
        assert_eq!(priority_for(&TaskKind::Other, "ci_activity"), 50);
        assert_eq!(priority_for(&TaskKind::Other, "review_requested"), 100);
    }

    #[test]
    fn should_process_reason_accepts_actionable_reasons_only() {
        assert!(should_process_reason("review_requested"));
        assert!(should_process_reason("mention"));
        assert!(should_process_reason("team_mention"));
        assert!(should_process_reason("comment"));
        assert!(should_process_reason("assign"));
        assert!(should_process_reason("author"));
        assert!(should_process_reason("manual"));

        assert!(!should_process_reason("subscribed"));
        assert!(!should_process_reason("ci_activity"));
        assert!(!should_process_reason(""));
    }

    #[test]
    fn task_kind_string_round_trip() {
        for kind in [
            TaskKind::ReviewRequest,
            TaskKind::Mention,
            TaskKind::Comment,
            TaskKind::AssignedIssue,
            TaskKind::AssignedPullRequest,
            TaskKind::Discussion,
            TaskKind::Other,
        ] {
            let as_str = kind.as_str();
            let restored = TaskKind::from_str(as_str).expect("round trip");
            assert_eq!(restored, kind);
        }
        assert!(TaskKind::from_str("not_a_kind").is_none());
    }
}
