pub type Affine = k256::AffinePoint;
pub type Projective = k256::ProjectivePoint;

pub use super::poly_commitment::AffinePolynomialCommitment;

macro_rules! use_frost_types {
    ($type: ident) => {
        pub type $type = frost_core::$type<frost_secp256k1::Secp256K1Sha256>;
    };
    ($path: ident :: $type: ident) => {
        pub type $type = frost_core::$path::$type<frost_secp256k1::Secp256K1Sha256>;
    };
    ($type: ident as $alias: ident) => {
        pub type $alias = frost_core::$type<frost_secp256k1::Secp256K1Sha256>;
    };
    ($path:ident :: $type: ident as $alias: ident) => {
        pub type $alias = frost_core::$path::$type<frost_secp256k1::Secp256K1Sha256>;
    };
    ($($type: ident $(as $alias: ident)?),*) => {
        $(use_frost_types!($type $(as $alias)?);)*
    };
    ($path: ident :: {$($type: ident $(as $alias: ident)?),*}) => {
        $(use_frost_types!($path::$type $(as $alias)?);)*
    };
}

use_frost_types!(
    Identifier,
    Element,
    Scalar,
    SigningPackage,
    Signature,
    VerifyingKey,
    BindingFactorList,
    GroupCommitment,
    Challenge
);

use_frost_types!(round1::{
    Nonce,
    NonceCommitment,
    SigningNonces,
    SigningCommitments
});

use_frost_types!(round2::SignatureShare);

use_frost_types!(keys::{
    KeyPackage,
    PublicKeyPackage,
    SigningShare,
    VerifyingShare,
    CoefficientCommitment,
    VerifiableSecretSharingCommitment as PolynomialCommitment
});
