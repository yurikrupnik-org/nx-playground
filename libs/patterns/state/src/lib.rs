#![allow(dead_code)]
// This trait defines what a traffic light state *can* do.
// `next_state` borrows instead of consuming, so a transition is just a
// reassignment behind `&mut self`. Works because the states hold no data;
// if they did, you'd want `self: Box<Self>` plus an `Option` field.
pub trait TrafficLightState {
  fn status(&self);
  fn next_state(&self) -> Box<dyn TrafficLightState>;
}

// Our first Concrete State: Red
pub struct RedLight;
impl TrafficLightState for RedLight {
  fn status(&self) {
    println!("Light is RED. STOP! 🛑");
  }

  fn next_state(&self) -> Box<dyn TrafficLightState> {
    println!("Changing from Red to Green... 🚦");
    Box::new(GreenLight)
  }
}

pub struct GreenLight;
impl TrafficLightState for GreenLight {
  fn status(&self) {
    println!("Light is GREEN. GO! 🟢");
  }

  fn next_state(&self) -> Box<dyn TrafficLightState> {
    println!("Changing from Green to Yellow... 🚦");
    Box::new(YellowLight)
  }
}

pub struct YellowLight;
impl TrafficLightState for YellowLight {
  fn status(&self) {
    println!("Light is YELLOW. PREPARE TO STOP! 🟠");
  }

  fn next_state(&self) -> Box<dyn TrafficLightState> {
    println!("Changing from Yellow to Red... 🚦");
    Box::new(RedLight)
  }
}

// This is our main TrafficLight struct, it holds the current state
pub struct TrafficLight {
  state: Box<dyn TrafficLightState>,
}

impl TrafficLight {
  pub fn new() -> Self {
    TrafficLight { state: Box::new(RedLight) }
  }

  pub fn change_state(&mut self) {
    self.state = self.state.next_state();
  }

  pub fn report_status(&self) {
    self.state.status();
  }
}

impl Default for TrafficLight {
  fn default() -> Self {
    Self::new()
  }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
      let mut light = TrafficLight::new();
      light.report_status();
      light.change_state();
      light.report_status();
      light.change_state();
      light.report_status();
      light.change_state();
      light.report_status();
    }
}
