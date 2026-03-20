//! Data ingestion from various file formats.

use crate::error::AnalyticsResult;
use polars::prelude::*;
use std::path::Path;

/// Data ingestor supporting multiple file formats.
pub struct Ingestor;

impl Ingestor {
    /// Read a CSV file into a DataFrame.
    #[cfg(feature = "csv")]
    pub fn read_csv(path: impl AsRef<Path>) -> AnalyticsResult<DataFrame> {
        let df = CsvReadOptions::default()
            .with_has_header(true)
            .with_infer_schema_length(Some(100))
            .try_into_reader_with_file_path(Some(path.as_ref().into()))?
            .finish()?;
        Ok(df)
    }

    /// Read a CSV file into a LazyFrame for deferred execution.
    #[cfg(feature = "csv")]
    pub fn scan_csv(path: impl AsRef<Path>) -> AnalyticsResult<LazyFrame> {
        let lf = LazyCsvReader::new(path.as_ref())
            .with_has_header(true)
            .with_infer_schema_length(Some(100))
            .finish()?;
        Ok(lf)
    }

    /// Read a JSON file into a DataFrame.
    #[cfg(feature = "json")]
    pub fn read_json(path: impl AsRef<Path>) -> AnalyticsResult<DataFrame> {
        let file = std::fs::File::open(path)?;
        let reader = std::io::BufReader::new(file);
        let df = JsonReader::new(reader).finish()?;
        Ok(df)
    }

    /// Read a Parquet file into a DataFrame.
    #[cfg(feature = "parquet")]
    pub fn read_parquet(path: impl AsRef<Path>) -> AnalyticsResult<DataFrame> {
        let file = std::fs::File::open(path)?;
        let df = ParquetReader::new(file).finish()?;
        Ok(df)
    }

    /// Scan a Parquet file as a LazyFrame.
    #[cfg(feature = "parquet")]
    pub fn scan_parquet(path: impl AsRef<Path>) -> AnalyticsResult<LazyFrame> {
        let args = ScanArgsParquet::default();
        let lf = LazyFrame::scan_parquet(path.as_ref(), args)?;
        Ok(lf)
    }

    /// Create a DataFrame from raw JSON string.
    #[cfg(feature = "json")]
    pub fn from_json_str(json: &str) -> AnalyticsResult<DataFrame> {
        let cursor = std::io::Cursor::new(json.as_bytes());
        let df = JsonReader::new(cursor).finish()?;
        Ok(df)
    }

    /// Write a DataFrame to CSV.
    #[cfg(feature = "csv")]
    pub fn write_csv(df: &mut DataFrame, path: impl AsRef<Path>) -> AnalyticsResult<()> {
        let file = std::fs::File::create(path)?;
        CsvWriter::new(file).finish(df)?;
        Ok(())
    }

    /// Write a DataFrame to Parquet.
    #[cfg(feature = "parquet")]
    pub fn write_parquet(df: &mut DataFrame, path: impl AsRef<Path>) -> AnalyticsResult<()> {
        let file = std::fs::File::create(path)?;
        ParquetWriter::new(file).finish(df)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_read_sales_csv() {
        let df = Ingestor::read_csv("data/sales.csv").unwrap();
        assert!(df.height() > 0);
        assert!(df.get_column_names().contains(&&PlSmallStr::from("order_id")));
        assert!(df.get_column_names().contains(&&PlSmallStr::from("quantity")));
        assert!(df.get_column_names().contains(&&PlSmallStr::from("unit_price")));
    }

    #[test]
    fn test_read_customers_csv() {
        let df = Ingestor::read_csv("data/customers.csv").unwrap();
        assert!(df.height() > 0);
        assert!(df.get_column_names().contains(&&PlSmallStr::from("customer_id")));
        assert!(df.get_column_names().contains(&&PlSmallStr::from("tier")));
    }

    #[test]
    fn test_scan_csv_lazy() {
        let lf = Ingestor::scan_csv("data/sales.csv").unwrap();
        let df = lf.collect().unwrap();
        assert!(df.height() > 0);
    }
}
