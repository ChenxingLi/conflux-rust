pub enum DpssError {
    InvalidReshareCommitment,
    DkgStageNotComplete,
    DkgStageHasFinished,
    EnoughReshareSubmit,
    LastEpochNotComplete,
    IncorrectHandoffLength,
    IncorrectHandoffShare,
    IncorrectHandoffSender,
    DuplicatedHandoffShare,
}
