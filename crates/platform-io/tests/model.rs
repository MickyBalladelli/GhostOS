use synos_platform_io::{AsyncQueue, Error};

#[test]
fn queue_saturation_keeps_every_submitted_slot_bounded() {
    let mut queue = AsyncQueue::<u8, u8, 2>::new();
    let first = queue.submit(10).unwrap();
    let second = queue.submit(20).unwrap();

    assert_eq!(queue.pending(), 2);
    assert_eq!(queue.submit(30), Err(Error::QueueFull));

    let first_submission = queue.dispatch().unwrap();
    queue.complete(first_submission.token, 11).unwrap();
    assert_eq!(queue.pending(), 2);
    assert_eq!(queue.submit(30), Err(Error::QueueFull));

    assert_eq!(queue.poll().unwrap().token, first);
    assert_eq!(queue.pending(), 1);
    assert!(queue.submit(30).is_ok());
    assert_ne!(first, second);
}

#[test]
fn duplicate_completion_is_rejected_and_published_once() {
    let mut queue = AsyncQueue::<u8, u8, 1>::new();
    queue.submit(7).unwrap();
    let submission = queue.dispatch().unwrap();

    assert_eq!(queue.complete(submission.token, 8), Ok(()));
    assert_eq!(queue.complete(submission.token, 9), Err(Error::RequestNotDispatched));
    assert_eq!(queue.poll().unwrap().result, 8);
    assert_eq!(queue.poll(), None);
}

#[test]
fn cancellation_races_follow_the_request_state() {
    let mut queue = AsyncQueue::<u8, u8, 1>::new();
    let queued = queue.submit(1).unwrap();
    assert_eq!(queue.cancel(queued), Ok(()));
    assert_eq!(queue.cancel(queued), Err(Error::InvalidToken));
    assert_eq!(queue.dispatch(), None);

    queue.submit(2).unwrap();
    let dispatched = queue.dispatch().unwrap();
    assert_eq!(queue.cancel(dispatched.token), Err(Error::CannotCancel));
    queue.complete(dispatched.token, 3).unwrap();
    assert_eq!(queue.cancel(dispatched.token), Err(Error::CannotCancel));
    assert_eq!(queue.poll().unwrap().result, 3);
}

#[test]
fn stale_generation_cannot_complete_a_reused_slot() {
    let mut queue = AsyncQueue::<u8, u8, 1>::new();
    let stale = queue.submit(1).unwrap();
    queue.cancel(stale).unwrap();
    let current = queue.submit(2).unwrap();

    assert_ne!(stale, current);
    assert_eq!(queue.complete(stale, 99), Err(Error::InvalidToken));

    let submission = queue.dispatch().unwrap();
    assert_eq!(submission.token, current);
    queue.complete(current, 2).unwrap();
    assert_eq!(queue.poll().unwrap().result, 2);
}
