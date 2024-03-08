mod element_matrix;
mod interpolate;
mod poly_commitment;
pub mod types;

pub use element_matrix::ElementMatrix;
pub use interpolate::{
    interpolate_and_evaluate_points, interpolate_and_evaluate_share,
};
pub use poly_commitment::{
    add_commitment, evaluate_commitment_points,
    generate_polynomial_commitments, validate_verifiable_secret_share,
    AffinePolynomialCommitment,
};
