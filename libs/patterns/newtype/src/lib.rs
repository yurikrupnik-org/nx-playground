#![allow(dead_code)]
#[derive(Debug, PartialEq, Eq)]
struct UserId(u64);

#[derive(Debug, PartialEq, Eq)]
struct ProductId(u64);

#[derive(Debug, PartialEq, Eq)]
struct OrderId(u64);

#[derive(Debug, PartialEq, Eq)]
struct Email(String);

use validator::ValidateEmail;
impl Email {
    pub fn new(value: String) -> Result<Self, &'static str> {
        if value.validate_email() {
            Ok(Self(value))
        } else {
            Err("invalid email address")
        }
    }
}

// A function that, you know, *really* needs a UserId
fn get_user_profile(user_id: UserId) {
    println!("Fetching profile for user ID: {user_id:?}");
}
// And this one, it *really* needs a ProductId
fn get_product_details(product_id: ProductId) {
    println!("Fetching details for product ID: {product_id:?}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
        let my_user_id = UserId(123);
        let my_product_id = ProductId(456);
        let my_order_id = OrderId(789);
        get_user_profile(my_user_id);
        get_product_details(my_product_id);
        // See? This would actually cause a compile-time error! How neat is that?
        // get_user_profile(my_product_id);
        // ^ expected struct `UserId`, found struct `ProductId`
        println!("Order ID: {my_order_id:?}");
        let email = Email::new("alice@example.com".to_string()).expect("invalid email address");
        assert_eq!(email, Email("alice@example.com".to_string()));
    }
}
