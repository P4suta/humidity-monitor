use switchbot_meter::{AlertState, Event, Policy};

fn policy() -> Policy {
    Policy {
        high_above: 55,
        high_seconds: 1800,
        recovery_seconds: 300,
        loss_seconds: 600,
    }
}

#[test]
fn thresholds_use_complete_intervals_and_confirming_readings() {
    let mut state = AlertState::new(0, policy());
    for at in (0..=1800).step_by(60) {
        assert!(state.advance(at, Some(55)).is_empty());
    }
    for at in (1801..3601).step_by(60) {
        assert!(state.advance(at, Some(56)).is_empty());
    }
    assert!(state.advance(3600, Some(56)).is_empty());
    assert!(state.advance(3601, None).is_empty());
    assert_eq!(state.advance(3601, Some(56)), [Event::High]);
    assert!(state.advance(3602, Some(60)).is_empty());
    for at in (3603..3903).step_by(60) {
        assert!(state.advance(at, Some(55)).is_empty());
    }
    assert!(state.advance(3902, Some(55)).is_empty());
    assert_eq!(state.advance(3903, Some(55)), [Event::Normal]);
    assert!(state.advance(3904, Some(34)).is_empty());
}

#[test]
fn missing_minutes_and_opposite_values_interrupt_both_timers() {
    let mut state = AlertState::new(0, policy());
    for at in (0..=1740).step_by(60) {
        assert!(state.advance(at, Some(56)).is_empty());
    }
    for at in (1860..3660).step_by(60) {
        assert!(state.advance(at, Some(56)).is_empty());
    }
    assert!(state.advance(3659, Some(55)).is_empty());
    for at in (3660..5460).step_by(60) {
        assert!(state.advance(at, Some(56)).is_empty());
    }
    assert_eq!(state.advance(5460, Some(56)), [Event::High]);
    for at in (5461..=5701).step_by(60) {
        assert!(state.advance(at, Some(54)).is_empty());
    }
    for at in (5821..6121).step_by(60) {
        assert!(state.advance(at, Some(54)).is_empty());
    }
    assert!(state.advance(6120, Some(56)).is_empty());
    for at in (6121..6421).step_by(60) {
        assert!(state.advance(at, Some(54)).is_empty());
    }
    assert_eq!(state.advance(6421, Some(54)), [Event::Normal]);
}

#[test]
fn loss_and_recovery_include_the_first_ten_minutes_and_late_arrivals() {
    let mut state = AlertState::new(90, policy());
    assert!(state.advance(689, None).is_empty());
    assert_eq!(state.advance(690, None), [Event::Lost]);
    assert!(state.advance(790, None).is_empty());
    assert_eq!(state.advance(800, Some(34)), [Event::Received]);
    assert!(state.advance(1399, None).is_empty());
    assert_eq!(
        state.advance(1400, Some(40)),
        [Event::Lost, Event::Received]
    );
    assert!(state.advance(1999, None).is_empty());
    assert_eq!(state.advance(2000, None), [Event::Lost]);
}

#[test]
fn thresholds_belong_to_the_installation() {
    let mut state = AlertState::new(
        10,
        Policy {
            high_above: 80,
            high_seconds: 5,
            recovery_seconds: 2,
            loss_seconds: 9,
        },
    );
    assert!(state.advance(10, Some(80)).is_empty());
    assert!(state.advance(11, Some(81)).is_empty());
    assert_eq!(state.advance(16, Some(81)), [Event::High]);
    assert!(state.advance(17, Some(80)).is_empty());
    assert_eq!(state.advance(19, Some(80)), [Event::Normal]);
    assert_eq!(state.advance(28, None), [Event::Lost]);
}
