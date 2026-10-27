use msdk::usage::{Quota, QuotaUnit, Scope, Source, State, Tokens};
use std::collections::HashSet;

#[test]
fn used_and_remaining_are_explicit_and_expired_windows_are_not_fresh() {
    let used = Quota::new(2500, QuotaUnit::Used, 100, Some(200)).unwrap();
    let remaining = Quota::new(7500, QuotaUnit::Remaining, 100, Some(200)).unwrap();
    assert_eq!(used.used_basis_points(), remaining.used_basis_points());
    assert_eq!(used.remaining_basis_points(), 7500);
    assert_eq!(used.observed_at(), 100);
    assert_eq!(used.state_at(State::Fresh, 199), State::Fresh);
    assert_eq!(used.state_at(State::Fresh, 200), State::Expired);
    assert_eq!(used.state_at(State::Stale, 200), State::Stale);
    assert!(Quota::new(10001, QuotaUnit::Used, 0, None).is_none());
    assert_eq!(Quota::new(0, QuotaUnit::Used, 0, None).unwrap().used_basis_points(), 0);
}

#[test]
fn token_subsets_are_not_counted_twice_and_missing_is_not_zero() {
    let counts = Tokens { input: Some(40), output: Some(25), cached_input: Some(12), reasoning: Some(7) };
    assert_eq!(counts.total(), Some(65));
    assert!(counts.subsets_valid());
    assert!(!Tokens { reasoning: Some(26), ..counts }.subsets_valid());
    assert!(!Tokens { cached_input: Some(41), ..counts }.subsets_valid());
    assert_eq!(Tokens { output: None, ..counts }.total(), None);
    assert_eq!(Tokens { input: Some(u64::MAX), output: Some(1), ..counts }.total(), None);
}

#[test]
fn account_aggregate_and_remote_sources_have_distinct_identity() {
    let aggregate = Scope { provider: "codex".into(), source: Source::LocalClient, account: None };
    let first = Scope { account: Some("account-a".into()), ..aggregate.clone() };
    let second = Scope { account: Some("account-b".into()), ..aggregate.clone() };
    let remote = Scope { source: Source::Server("server-a".into()), ..first.clone() };
    assert!(!format!("{remote:?}").contains("server-a"));
    assert!(!format!("{remote:?}").contains("account-a"));
    assert_eq!([aggregate, first, second, remote].into_iter().collect::<HashSet<_>>().len(), 4);
}

#[test]
fn snapshots_retain_distinct_accounts_and_reject_duplicate_or_invalid_data() {
    use msdk::usage::{Coverage, Observation, Snapshot, SnapshotError, Window};
    let scope =
        Scope { provider: "claude".into(), source: Source::Server("paired".into()), account: Some("one".into()) };
    let first = Observation {
        scope: scope.clone(),
        provider_label: "Claude".into(),
        source_label: "Paired server".into(),
        account_label: Some("Work account".into()),
        selected: true,
        state: State::Fresh,
        observed_at: Some(100),
        windows: vec![Window {
            key: "week".into(),
            label: "7 day".into(),
            quota: Some(Quota::new(0, QuotaUnit::Used, 100, None).unwrap()),
        }],
        tokens: None,
    };
    let second = Observation {
        scope: Scope { account: Some("two".into()), ..scope },
        account_label: Some("Personal account".into()),
        selected: false,
        ..first.clone()
    };
    let mut snapshot = Snapshot { coverage: Coverage::Partial, observations: vec![first.clone(), second] };
    assert_eq!(Snapshot::VERSION, 1);
    assert_eq!(snapshot.validate(), Ok(()));
    let mut aged = snapshot.clone();
    aged.refresh_freshness(401, 300);
    assert_eq!(aged.observations[0].state, State::Stale);
    aged.refresh_freshness(100, 300);
    assert_eq!(aged.observations[0].state, State::Stale, "freshness checks never promote stale data");
    let mut future = snapshot.clone();
    future.refresh_freshness(99, 300);
    assert_eq!(future.observations[0].state, State::Unknown);
    let grouped = snapshot.groups().unwrap();
    assert_eq!(grouped.len(), 1);
    assert_eq!(grouped[0].label, "Claude");
    assert_eq!(grouped[0].accounts.len(), 2);
    assert_eq!(grouped[0].accounts[1].account_label.as_deref(), Some("Personal account"));
    assert!(!grouped[0].accounts[1].selected);
    snapshot.observations[1].provider_label = "Other name".into();
    assert_eq!(snapshot.validate(), Err(SnapshotError::InconsistentProviderLabel));
    snapshot.observations[1].provider_label = "Claude".into();
    snapshot.observations[0].account_label = None;
    assert_eq!(snapshot.validate(), Err(SnapshotError::InvalidLabel));
    snapshot.observations[0].account_label = Some("Work account".into());
    snapshot.observations.push(first.clone());
    assert_eq!(snapshot.validate(), Err(SnapshotError::DuplicateScope));
    snapshot.observations.pop();
    let duplicate = snapshot.observations[0].windows[0].clone();
    snapshot.observations[0].windows.push(duplicate);
    assert_eq!(snapshot.validate(), Err(SnapshotError::DuplicateWindow));
    snapshot.observations[0].windows.pop();
    snapshot.observations[0].observed_at = None;
    assert_eq!(snapshot.validate(), Err(SnapshotError::FreshWithoutTimestamp));
    snapshot.observations[0].observed_at = Some(101);
    assert_eq!(snapshot.validate(), Ok(()), "window and summary source timestamps may differ");
    snapshot.observations[0].observed_at = Some(100);
    snapshot.observations[0].tokens = Some(Tokens { input: Some(1), cached_input: Some(2), ..Tokens::default() });
    assert_eq!(snapshot.validate(), Err(SnapshotError::InvalidTokenSubset));
}
