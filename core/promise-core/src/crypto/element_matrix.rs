use std::{
    collections::{BTreeMap, BTreeSet},
    ops::{Index, IndexMut},
};

use super::{
    interpolate_and_evaluate_points,
    types::{Element, PolynomialCommitment},
};

pub struct ElementMatrix {
    rows: usize,
    cols: usize,
    inner: Vec<Element>,
}

impl ElementMatrix {
    pub fn new(rows: usize, cols: usize) -> Self {
        let zero = Element::IDENTITY;
        Self {
            rows,
            cols,
            inner: vec![zero; rows * cols],
        }
    }

    pub fn create_empty(&self) -> Self { Self::new(self.rows, self.cols) }

    pub fn add_row(&mut self, row_idx: usize, pc: PolynomialCommitment) {
        assert!(row_idx < self.rows);
        assert!(pc.coefficients().len() <= self.cols);

        for col_idx in 0..self.cols {
            self[(col_idx, row_idx)] += pc.coefficients()[col_idx].value();
        }
    }

    pub fn interpolate_col(&mut self, col_idx: usize, filled: BTreeSet<usize>) {
        let filled_points: BTreeMap<usize, Element> = filled
            .into_iter()
            .map(|x| (x, self[(col_idx, x)].clone()))
            .collect();
        let evaluated_points =
            interpolate_and_evaluate_points(filled_points, 0..self.rows);
        for (row_idx, elem) in evaluated_points.into_iter() {
            self[(col_idx, row_idx)] = elem;
        }
    }
}

impl Index<(usize, usize)> for ElementMatrix {
    type Output = Element;

    fn index(&self, (col_idx, row_idx): (usize, usize)) -> &Self::Output {
        &self.inner[col_idx * self.rows + row_idx]
    }
}

impl IndexMut<(usize, usize)> for ElementMatrix {
    fn index_mut(
        &mut self, (col_idx, row_idx): (usize, usize),
    ) -> &mut Self::Output {
        &mut self.inner[col_idx * self.rows + row_idx]
    }
}
