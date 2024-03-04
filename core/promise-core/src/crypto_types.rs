use frost_secp256k1::Secp256K1Sha256;

pub type Identifier = frost_core::Identifier<Secp256K1Sha256>;
pub type SignatureShare = frost_core::round2::SignatureShare<Secp256K1Sha256>;
pub type SigningCommitments =
    frost_core::round1::SigningCommitments<Secp256K1Sha256>;
pub type NonceCommitment = frost_core::round1::NonceCommitment<Secp256K1Sha256>;
pub type Nonce = frost_core::round1::Nonce<Secp256K1Sha256>;
pub type SigningNonces = frost_core::round1::SigningNonces<Secp256K1Sha256>;

pub type PolynomialCommitment =
    frost_core::keys::VerifiableSecretSharingCommitment<Secp256K1Sha256>;
pub type Element = frost_core::Element<Secp256K1Sha256>;
pub type Scalar = frost_core::Scalar<Secp256K1Sha256>;
pub type SigningShare = frost_core::keys::SigningShare<Secp256K1Sha256>;
pub type Challenge = frost_core::Challenge<Secp256K1Sha256>;

pub type KeyPackage = frost_core::keys::KeyPackage<Secp256K1Sha256>;
pub type PublicKeyPackage = frost_core::keys::PublicKeyPackage<Secp256K1Sha256>;
pub type SigningPackage = frost_core::SigningPackage<Secp256K1Sha256>;
pub type Signature = frost_core::Signature<Secp256K1Sha256>;

pub type VerifyingKey = frost_core::VerifyingKey<Secp256K1Sha256>;
pub type VerifyingShare = frost_core::keys::VerifyingShare<Secp256K1Sha256>;
pub type BindingFactorList = frost_core::BindingFactorList<Secp256K1Sha256>;
pub type GroupCommitment = frost_core::GroupCommitment<Secp256K1Sha256>;
