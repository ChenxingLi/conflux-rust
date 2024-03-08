#[derive(PartialEq, Eq, Debug)]
pub enum FrostError {
    NotEnoughSigningShares,
    TooLatePreCommit,
    EpochNotStart,
    EjectedNode,
    UnknownSignTask,
    UnknownSigner,
    UnknownNodeID,
    InvalidSignatureShare,
    DuplicatedSignatureShare,
    IdentityNonceCommitment,
    InconsistentNonceCommitment,
}
