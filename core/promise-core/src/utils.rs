//! Extracted from [ark_std](https://github.com/arkworks-rs/std)

/// Creates parallel iterator over refs if `parallel` feature is enabled.
/// Additionally, if the object being iterated implements
/// `IndexedParallelIterator`, then one can specify a minimum size for
/// iteration.
#[macro_export]
macro_rules! cfg_iter {
    ($e:expr, $min_len:expr) => {{
        #[cfg(feature = "parallel")]
        let result = $e.par_iter().with_min_len($min_len);

        #[cfg(not(feature = "parallel"))]
        let result = $e.iter();

        result
    }};
    ($e:expr) => {{
        #[cfg(feature = "parallel")]
        let result = $e.par_iter();

        #[cfg(not(feature = "parallel"))]
        let result = $e.iter();

        result
    }};
}

#[macro_export]
macro_rules! cfg_into_iter {
    ($e:expr, $min_len:expr) => {{
        #[cfg(feature = "parallel")]
        let result = $e.into_par_iter().with_min_len($min_len);

        #[cfg(not(feature = "parallel"))]
        let result = $e.into_iter();

        result
    }};
    ($e:expr) => {{
        #[cfg(feature = "parallel")]
        let result = $e.into_par_iter();

        #[cfg(not(feature = "parallel"))]
        let result = $e.into_iter();

        result
    }};
}
