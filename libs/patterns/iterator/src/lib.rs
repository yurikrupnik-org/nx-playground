#![allow(dead_code)]
struct MyRange {
  current: u32,
  end: u32,
}
impl Iterator for MyRange {
  type Item = u32;
  fn next(&mut self) -> Option<Self::Item> {
    if self.current < self.end {
      let result = self.current;
      self.current += 1;
      Some(result)
    } else {
      None
    }
  }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
      let my_iterator = MyRange { current: 1, end: 5 };
      let sum = my_iterator.sum::<u32>();
      assert_eq!(sum, 10);
    }
}
