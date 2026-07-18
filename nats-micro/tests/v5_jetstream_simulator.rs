#![cfg(feature = "test-jetstream")]

use std::time::Duration;

use nats_micro::{ConsumerAction, Text, message, service, testing::TestApp};

#[message]
pub struct UserEvent<'a> {
    pub user_id: &'a str,
}

#[message]
pub struct OwnedUserEvent {
    pub user_id: String,
}

#[service(name = "simulated-consumers", version = "1.0.0")]
impl SimulatedConsumerService {
    #[consumer(
        stream = "USERS",
        durable = "welcome-email",
        filter = "users.events.created",
        concurrency = 2,
        ack_wait = "2s",
        max_deliver = 3
    )]
    async fn welcome(
        event: UserEvent<'_>,
        #[header("Nats-Num-Delivered")] attempt: &str,
    ) -> ConsumerAction {
        let _ = event.user_id;
        if attempt == "1" {
            ConsumerAction::NackAfter(Duration::from_secs(5))
        } else {
            ConsumerAction::Ack
        }
    }

    #[consumer(
        stream = "USERS",
        durable = "always-fails",
        filter = "users.events.failed",
        ack_wait = "2s",
        max_deliver = 3
    )]
    async fn always_fails(event: UserEvent<'_>) -> ConsumerAction {
        let _ = event;
        ConsumerAction::Nack
    }

    #[consumer(
        stream = "USERS",
        durable = "terminal",
        filter = "users.events.terminal",
        ack_wait = "2s"
    )]
    async fn terminal(body: Text<'_>) -> ConsumerAction {
        let _ = body;
        ConsumerAction::Term
    }

    #[consumer(
        stream = "USERS",
        durable = "backoff",
        filter = "users.events.backoff",
        ack_wait = "2s",
        backoff = ["5s", "10s"]
    )]
    async fn backoff(event: UserEvent<'_>) -> ConsumerAction {
        let _ = event;
        ConsumerAction::Ack
    }

    #[consumer(
        stream = "USERS",
        durable = "ack-wait",
        filter = "users.events.ack-wait",
        ack_wait = "3s"
    )]
    async fn ack_wait(event: UserEvent<'_>) -> ConsumerAction {
        let _ = event;
        ConsumerAction::Ack
    }
}

fn app()
-> nats_micro::testing::TestHarness<(), nats_micro::Cons<SimulatedConsumerService, nats_micro::Nil>>
{
    TestApp::stateless()
        .jetstream(|jetstream| {
            jetstream.stream("USERS", ["users.events.>"]);
            jetstream.dead_letter("always-fails", "dead.users.failed");
        })
        .serve(SimulatedConsumerService)
        .start()
}

#[nats_micro::test(start_paused = true)]
async fn delayed_nack_retries_only_after_manual_time() -> nats_micro::Result<()> {
    let app = app();
    assert!(
        app.jetstream()
            .publish("outside.declared.streams", "ignored")
            .await
            .is_err()
    );
    app.jetstream()
        .publish_json("users.events.created", &UserEvent { user_id: "user-7" })
        .await?;

    app.run_until_idle().await?;
    assert_eq!(app.deliveries("welcome-email"), 1);
    assert_eq!(app.deliveries("always-fails"), 0);
    assert_eq!(app.deliveries("terminal"), 0);
    assert_eq!(app.jetstream().pending(), 1);

    app.clock().advance(Duration::from_secs(4)).await;
    app.run_until_idle().await?;
    assert_eq!(app.deliveries("welcome-email"), 1);

    app.clock().advance(Duration::from_secs(1)).await;
    app.run_until_idle().await?;
    assert_eq!(app.deliveries("welcome-email"), 2);
    assert_eq!(app.jetstream().pending(), 0);
    Ok(())
}

#[nats_micro::test(start_paused = true)]
async fn max_deliver_dead_letters_and_term_stops_delivery() -> nats_micro::Result<()> {
    let app = app();
    app.jetstream()
        .publish_json(
            "users.events.failed",
            &UserEvent {
                user_id: "failed-user",
            },
        )
        .await?;
    app.jetstream()
        .publish("users.events.terminal", "stop")
        .await?;

    app.run_until_idle().await?;
    assert_eq!(app.deliveries("always-fails"), 3);
    assert_eq!(app.deliveries("terminal"), 1);
    let dead_letter: OwnedUserEvent = app.events().subject("dead.users.failed").single_json()?;
    assert_eq!(dead_letter.user_id, "failed-user");
    assert_eq!(app.jetstream().pending(), 0);
    Ok(())
}

#[nats_micro::test(start_paused = true)]
async fn simulated_ack_wait_and_backoff_are_deterministic() -> nats_micro::Result<()> {
    let app = app();
    app.jetstream().timeout_next("backoff");
    app.jetstream()
        .publish_json("users.events.backoff", &UserEvent { user_id: "backoff" })
        .await?;
    app.run_until_idle().await?;
    assert_eq!(app.deliveries("backoff"), 1);

    app.clock().advance(Duration::from_secs(4)).await;
    app.run_until_idle().await?;
    assert_eq!(app.deliveries("backoff"), 1);
    app.clock().advance(Duration::from_secs(1)).await;
    app.run_until_idle().await?;
    assert_eq!(app.deliveries("backoff"), 2);

    app.jetstream().timeout_next("ack-wait");
    app.jetstream()
        .publish_json(
            "users.events.ack-wait",
            &UserEvent {
                user_id: "ack-wait",
            },
        )
        .await?;
    app.run_until_idle().await?;
    assert_eq!(app.deliveries("ack-wait"), 1);
    app.clock().advance(Duration::from_secs(2)).await;
    app.run_until_idle().await?;
    assert_eq!(app.deliveries("ack-wait"), 1);
    app.clock().advance(Duration::from_secs(1)).await;
    app.run_until_idle().await?;
    assert_eq!(app.deliveries("ack-wait"), 2);
    Ok(())
}
