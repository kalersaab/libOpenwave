use ringbuf::{
    HeapRb,
    traits::{Consumer, Observer, Producer, Split},
};

pub struct AudioRingBuffer {
    producer: ringbuf::HeapProd<f32>,
    consumer: ringbuf::HeapCons<f32>,
    capacity: usize,
}

impl AudioRingBuffer {
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0, "Ring buffer capacity must be positive");

        let ring = HeapRb::<f32>::new(capacity);
        let (producer, consumer) = ring.split();

        Self {
            producer,
            consumer,
            capacity,
        }
    }

    pub fn push(&mut self, sample: f32) -> Result<(), f32> {
        self.producer.try_push(sample)
    }

    pub fn pop(&mut self) -> Option<f32> {
        self.consumer.try_pop()
    }

    pub fn is_empty(&self) -> bool {
        self.consumer.is_empty()
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_and_reads_samples() {
        let mut buffer = AudioRingBuffer::new(4);

        assert!(buffer.push(0.25).is_ok());
        assert!(buffer.push(-0.5).is_ok());

        assert_eq!(buffer.pop(), Some(0.25));
        assert_eq!(buffer.pop(), Some(-0.5));
        assert_eq!(buffer.pop(), None);
    }

    #[test]
    fn detects_full_buffer() {
        let mut buffer = AudioRingBuffer::new(2);

        assert!(buffer.push(1.0).is_ok());
        assert!(buffer.push(2.0).is_ok());

        assert_eq!(buffer.push(3.0), Err(3.0));
    }

    #[test]
    fn detects_empty_buffer() {
        let buffer = AudioRingBuffer::new(4);
        assert!(buffer.is_empty());
    }
}
