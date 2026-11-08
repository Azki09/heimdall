use crate::format::SlotId;

/// Everything that can go wrong in the core.
///
/// Messages never include secret material. The variants are fine-grained so
/// the UI can tell "wrong password" apart from "wrong USB key" (§5.2,
/// `keyfile_hint`) and from a damaged file.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    // ---- Parsing (.heim / .bifrost) ----
    #[error("not a Heimdall vault (bad magic)")]
    BadMagic,
    #[error("unsupported vault format version {0}")]
    UnsupportedVersion(u16),
    #[error("reserved header flags are set")]
    ReservedFlags,
    #[error("the file is truncated or malformed")]
    Malformed,
    #[error("too many key slots ({0}, max {max})", max = crate::format::MAX_SLOTS)]
    TooManySlots(usize),
    #[error("slot has wrong length for its type")]
    BadSlotLength,
    #[error("Argon2id parameters out of the allowed range")]
    KdfParamsOutOfRange,
    #[error("not a Heimdall key file (bad magic or size)")]
    BadKeyfile,

    // ---- Unlocking ----
    #[error("wrong password")]
    WrongPassword,
    #[error("wrong recovery code")]
    WrongRecoveryCode,
    #[error("the recovery code is not valid (check the words)")]
    InvalidRecoveryCode,
    #[error("this vault requires its USB key")]
    KeyfileRequired,
    #[error("this USB key belongs to a different vault")]
    KeyfileForOtherVault,
    #[error("this USB key is not enrolled in this vault")]
    WrongKeyfile,
    #[error("the vault has no slot that can be opened this way")]
    NoUsableSlot,
    #[error("the vault content could not be decrypted (file damaged or tampered with)")]
    ContentAuthFailed,
    #[error("the vault content is malformed")]
    BadContent,
    #[error("unsupported content version {0}")]
    UnsupportedContentVersion(u16),

    // ---- Access management ----
    #[error("USB key for slot {0} must be connected")]
    MissingKeyfile(SlotId),
    #[error("slot {0} not found")]
    SlotNotFound(SlotId),
    #[error("the operation would leave the vault without a password slot")]
    NoPasswordSlotLeft,
    #[error("the recovery slot cannot be revoked; regenerate the code instead")]
    CannotRevokeRecovery,

    // ---- Export ----
    #[error("no notes were chosen")]
    NothingToExport,

    // ---- Crypto / OS ----
    #[error("cryptographic failure")]
    Crypto,
    #[error("the operating system random number generator failed")]
    Rng,
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
