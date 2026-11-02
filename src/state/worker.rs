//! One writer actor per scope, with bounded admission and commit acknowledgments.
use super::{Engine, Frame};
use anyhow::Result;
use tokio::sync::{mpsc, oneshot};

pub struct Request {
    pub frame: Frame,
    pub committed: oneshot::Sender<Result<bool>>,
}
pub fn spawn(
    engine: Engine,
    capacity: usize,
) -> Result<(mpsc::Sender<Request>, tokio::task::JoinHandle<()>)> {
    anyhow::ensure!(
        (1..=64).contains(&capacity),
        "state queue capacity must be 1..=64"
    );
    let (sender, mut receiver) = mpsc::channel::<Request>(capacity);
    let task = tokio::spawn(async move {
        while let Some(request) = receiver.recv().await {
            let result = engine.apply(&request.frame).await;
            let failed = result.is_err();
            let _ = request.committed.send(result);
            if failed {
                break;
            } // Do not advance past a rejected checkpoint.
        }
    });
    Ok((sender, task))
}
