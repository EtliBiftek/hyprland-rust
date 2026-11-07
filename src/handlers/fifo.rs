//! wp-fifo-v1 and wp-commit-timing-v1.
//! shortcuts: both are registered unmanaged: requests are accepted but commits
//! are never held back, so FIFO barriers and target timestamps have no effect
//! (clients just present as fast as frame callbacks allow).

use smithay::{
    delegate_commit_timing, delegate_fifo,
    reexports::wayland_server::DisplayHandle,
    wayland::{commit_timing::CommitTimingManagerState, fifo::FifoManagerState},
};

use crate::Hypr;

pub fn init(dh: &DisplayHandle) -> (FifoManagerState, CommitTimingManagerState) {
    (
        FifoManagerState::unmanaged::<Hypr>(dh),
        CommitTimingManagerState::unmanaged::<Hypr>(dh),
    )
}

delegate_fifo!(Hypr);
delegate_commit_timing!(Hypr);
