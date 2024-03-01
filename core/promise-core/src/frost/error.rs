#[derive(PartialEq, Eq, Debug)]
pub enum FrostError {
    NotEnoughUnusedPreCommit,
    NotEnoughVotes,
    TooLatePreCommit,
    EpochNotStart,
    EjectedNode,
    UnknownSignTask,
    UnknownSigner,
    UnknownNodeID,
    InvalidSignatureShare,
    DuplicatedSignatureShare,
    IdentityNonceCommitment,
}
