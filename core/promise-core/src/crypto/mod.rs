mod element_matrix;
mod poly_commitment;
pub mod types;

pub use element_matrix::ElementMatrix;
pub use poly_commitment::{
    add_commitment, generate_polynomial_commitments,
    interpolate_and_evaluate_points, validate_verifiable_secret_share,
    AffinePolynomialCommitment,
};
