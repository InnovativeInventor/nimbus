use log::error;

#[macro_export]
macro_rules! unhandled {
    () => {
        // ()
        // error!("Unhandled error!")
        panic!("not handled")
    };
    ($($arg:tt)+) => {
        // ()
        // error!("Unhandled error!")
        panic!("not handled: {}", format_args!($($arg)+))
    };
}

// #[macro_export]
// macro_rules! breakpoint {
//     (_) => {
//         unsafe {
//             std::intrinsics::breakpoint();
//         }
//     };
// }
