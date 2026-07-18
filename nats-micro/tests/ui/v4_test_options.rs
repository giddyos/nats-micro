#[nats_micro::test(start_paused = true)]
async fn paused() {}

#[nats_micro::test(flavor = "multi_thread", worker_threads = 2)]
async fn multi_thread() {}

fn main() {}
