fn first_word(s: &str) -> &str {
    match s.find(' ') {
        Some(index) => &s[..index],
        None => s,
    }
}

fn step3_borrowing() {
    let sentence = String::from("hello world");
    let word = first_word(&sentence);
    println!("{word}"); // must print: hello
    println!("{sentence}"); // must still compile and print: hello world
}

fn parse_and_double(input: &str) -> Result<i32, std::num::ParseIntError> {
    let n: i32 = input.parse()?;
    Ok(n * 2)
}

fn find_first_even(nums: &[i32]) -> Option<i32> {
    for &i in nums {
        if i % 2 == 0 {
            return Some(i);
        }
    }
    None
}

fn step4_option_result() {
    match parse_and_double("abc") {
        Ok(num) => println!("{num}"),
        Err(e) => println!("{}", e),
    }

    match find_first_even(&[1, 2, 4, 7]) {
        Some(num) => println!("{num}"),
        None => println!("no even number"),
    }

    println!("{:?}", parse_and_double("21")); // expect Ok(42)
    println!("{:?}", parse_and_double("-5")); // expect Ok(-10)
    println!("{:?}", find_first_even(&[1, 3, 5])); // expect None
}

enum Shape {
    Circle { radius: f64 },
    Rectangle { width: f64, height: f64 },
}

trait Area {
    fn area(&self) -> f64;
}

impl Area for Shape {
    fn area(&self) -> f64 {
        match self {
            Self::Circle { radius } => std::f64::consts::PI * radius.powi(2),
            Self::Rectangle { width, height } => width * height,
        }
    }
}

fn step5_shapes() {
    let circle = Shape::Circle { radius: 12.0 };
    let rectangle = Shape::Rectangle {
        width: 12.0,
        height: 12.0,
    };

    println!("{}", circle.area());
    println!("{}", rectangle.area());
}

fn step6_byte_slicing() {
    let n: u32 = 4_29_49_67_295;
    let bytes: [u8; 4] = n.to_le_bytes();
    println!("{bytes:?}"); // [255, 255, 255, 255]
    let back = u32::from_le_bytes(bytes);
    assert_eq!(back, n);

    let buf = vec![1, 2, 3, 4, 255];
    let slice: &[u8] = &buf[1..3];
    let another_slice: [u8; 4] = buf[0..4].try_into().expect("Range 0..4 is exactly 4 bytes");
    let byte_32 = u32::from_le_bytes(another_slice);
    // u32::from_le_bytes(buf[0..4])
    println!("{byte_32:?}"); //67305985
    println!("{slice:?}"); // [2, 3]
}

fn main() {
    step3_borrowing();
    step4_option_result();
    step5_shapes();
    step6_byte_slicing();
}
