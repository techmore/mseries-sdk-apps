#[test]
fn initial_state_round_trips() {
    let state = app_model::State::initial().expect("typed initial state");
    let store = state.clone().into_store().unwrap();
    assert_eq!(app_model::State::from_store(store).unwrap(), state);
}
