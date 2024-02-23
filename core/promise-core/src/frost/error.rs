#[derive(PartialEq, Eq, Debug)]
pub enum FrostError {
    NotEnoughUnusedPreCommit,
    TooLatePreCommit,
    EpochNotStart,
    EjectedNodePreCommit,
    UnknownSignTask,
    UnknownSigner,
    InvalidSignatureShare,
    DuplicatedSignatureShare,
    IdentityNonceCommitment,
}
