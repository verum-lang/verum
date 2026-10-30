use super::*;

#[tokio::test]
async fn abandoned_request_cancels_queued_or_active_work_without_poisoning_the_next_request() {
    let (requests, mut receiver) = mpsc::channel(8);
    let language = Language { requests };
    let abandoned;
    {
        let request = language.request("draft.vr".into(), "fn first() {}".into(), "analyze", Value::Null);
        futures::pin_mut!(request);
        assert!(futures::poll!(&mut request).is_pending());
        abandoned = receiver.try_recv().unwrap();
        assert!(!abandoned.control.is_cancelled());
    }
    assert!(abandoned.control.is_cancelled());
    assert!(abandoned.reply.is_closed());
    let request = language.request("draft.vr".into(), "fn next() {}".into(), "analyze", Value::Null);
    futures::pin_mut!(request);
    assert!(futures::poll!(&mut request).is_pending());
    let next = receiver.try_recv().unwrap();
    assert!(!next.control.is_cancelled());
    next.reply.send(Ok(json!({"complete":true}))).unwrap();
    assert_eq!(request.await.unwrap(), json!({"complete":true}));
}
