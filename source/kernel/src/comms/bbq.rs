pub mod bidi;
pub mod mpsc;

// Possible bbqueues:
//
// * Bidi
// * A one way, mutex'd producer (mpsc async)
//   *
// * A one way, wait cell'd consumer (spsc half-async)
