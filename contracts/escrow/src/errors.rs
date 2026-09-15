use soroban_sdk::contracterror;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized = 1,
    NoMilestones = 2,
    InvalidAmount = 3,
    NotAuthorized = 4,
    InvalidState = 5,
    EscrowNotFound = 6,
    InvalidMilestone = 7,
    AlreadyReleased = 8,
    TooEarly = 9,
    EscrowDisputed = 10,
    NoDispute = 11,
    AlreadyResolved = 12,
    NotAJuror = 13,
    AlreadyVoted = 14,
    VotingIncomplete = 15,
    InsufficientStake = 16,
    NoJurorsAvailable = 17,
    FeeTooHigh = 18,
    InvalidJurySize = 19,
}
