/*!

Arithmetic encoder/decoder using the Range encoder underneath.
Can be used in a general case of entropy coding stage. Supposed to be fast.

# Links

http://en.wikipedia.org/wiki/Arithmetic_coding
http://en.wikipedia.org/wiki/Range_encoding

# Example
```rust
# #[allow(unused_must_use)];
use std::io::{MemWriter, MemReader};
use compress::entropy::ari;

// Encode some text
let text = "some text";
let mut e = ari::ByteEncoder::new(MemWriter::new());
e.write_str(text);
let (encoded, _) = e.finish();

// Decode the encoded text
let mut d = ari::ByteDecoder::new(MemReader::new(encoded.unwrap()));
let decoded = d.read_to_end().unwrap();
```
# Credit

This is an original implementation.

*/

use std::fmt::Show;
use std::io::IoResult;
use std::vec::Vec;
#[cfg(tune)]
use std::num;

pub use self::table::{ByteDecoder, ByteEncoder};

pub mod bin;
pub mod table;
#[cfg(test)]
mod test;

pub type Symbol = u8;
static SYMBOL_BITS: uint = 8;
static SYMBOL_TOTAL: uint = 1<<SYMBOL_BITS;

pub type Border = u32;
static BORDER_BITS: uint = 32;
static BORDER_EXCESS: uint = BORDER_BITS-SYMBOL_BITS;
static BORDER_SYMBOL_MASK: u32 = ((SYMBOL_TOTAL-1) << BORDER_EXCESS) as u32;


/// Range Encoder basic primitive
/// Gets probability ranges on the input, produces whole bytes of code on the output,
/// where the code is an arbitrary fixed-ppoint value inside the resulting probability range.
pub struct RangeEncoder {
    low: Border,
    hai: Border,
    /// The minimum distance between low and hai to keep at all times,
    /// has to be at least the largest incoming 'total',
    /// and optimally many times larger
    pub threshold: Border,
    // tune parameters
    bits_lost_on_threshold_cut: f32,
    bits_lost_on_division: f32,
}

impl RangeEncoder {
    /// Create a new instance
    /// will keep the active range below 'max_range'
    /// A typical value is 16k
    pub fn new(max_range: Border) -> RangeEncoder {
        assert!(max_range > (SYMBOL_TOTAL as Border));
        RangeEncoder {
            low: 0,
            hai: -1,
            threshold: max_range,
            bits_lost_on_threshold_cut: 0.0,
            bits_lost_on_division: 0.0,
        }
    }

    /// Reset the current range
    pub fn reset(&mut self) {
        self.low = 0;
        self.hai = -1;
    }

    #[cfg(tune)]
    fn count_bits(range: Border, total: Border) -> f32 {
        -num::log2((range as f32) / (total as f32))
    }

    #[cfg(not(tune))]
    fn count_bits(_range: Border, _total: Border) -> f32 {
        0.0
    }

    /// return the number of bits lost due to threshold cuts and integer operations
    #[cfg(tune)]
    pub fn get_bits_lost(&self) -> (f32, f32) {
        (self.bits_lost_on_threshold_cut, self.bits_lost_on_division)
    }

    /// Process a given interval [from/total,to/total) into the current range
    /// Yields stabilized code symbols (bytes) into the 'fn_shift' function
    pub fn process(&mut self, total: Border, from: Border, to: Border, fn_shift: |Symbol|) {
        let range = (self.hai - self.low) / total;
        assert!(range>0, "RangeCoder range is too narrow [{}-{}) for the total {}",
            self.low, self.hai, total);
        debug!("\t\tProcessing [{}-{})/{} with range {}", from, to, total, range);
        assert!(from < to);
        let mut lo = self.low + range*from;
        let mut hi = self.low + range*to;
        self.bits_lost_on_division += RangeEncoder::count_bits(range*total, self.hai-self.low);
        loop {
            if (lo^hi) & BORDER_SYMBOL_MASK != 0 {
                if hi-lo > self.threshold {
                    break
                }
                let old_range = hi-lo;
                let lim = hi & BORDER_SYMBOL_MASK;
                if hi-lim >= lim-lo {lo=lim}
                else {hi=lim-1};
                assert!(lo < hi);
                self.bits_lost_on_threshold_cut += RangeEncoder::count_bits(hi-lo, old_range);
            }

            debug!("\t\tShifting on [{}-{}) to symbol {}", lo, hi, lo>>BORDER_EXCESS);
            fn_shift((lo>>BORDER_EXCESS) as Symbol);
            lo<<=SYMBOL_BITS; hi<<=SYMBOL_BITS;
            assert!(lo < hi);
        }
        self.low = lo;
        self.hai = hi;
    }

    /// Query the value encoded by 'code' in range [0,total)
    pub fn query(&self, total: Border, code: Border) -> Border {
        debug!("\t\tQuerying code {} of total {} under range [{}-{})",
            code, total, self.low, self.hai);
        assert!(self.low <= code && code < self.hai)
        let range = (self.hai - self.low) / total;
        (code - self.low) / range
    }

    /// Get the code tail and close the range
    /// used at the end of encoding
    pub fn get_code_tail(&mut self) -> Border {
        let tail = self.low;
        self.low = 0;
        self.hai = 0;
        tail
    }
}


/// An abstract model to produce probability ranges
/// Can be a table, a mix of tables, or just a smart function.
pub trait Model<V> {
    /// get the probability range of a value
    fn get_range(&self, value: V) -> (Border,Border);
    /// find the value by a given probability offset, return with the range
    fn find_value(&self, offset: Border) -> (V,Border,Border);
    /// sum of all probabilities
    fn get_denominator(&self) -> Border;
}


/// Arithmetic coding functions
pub static RANGE_DEFAULT_THRESHOLD: Border = 1<<14;

/// Encode 'value', using a model and a range encoder
/// returns a list of output bytes
pub fn encode<V: Copy + Show, M: Model<V>>(value: V, model: &M, re: &mut RangeEncoder, accum: &mut Vec<Symbol>) {
    let (lo, hi) = model.get_range(value);
    let total = model.get_denominator();
    debug!("\tEncoding value {} of range [{}-{}) with total {}", value, lo, hi, total);
    re.process(total, lo, hi, |s| accum.push(s));
}

/// Decode a value using given 'code' on the range encoder
/// Returns a (value, num_symbols_to_shift) pair
pub fn decode<V: Copy + Show, M: Model<V>>(code: Border, model: &M, re: &mut RangeEncoder) -> (V,uint) {
    let total = model.get_denominator();
    let offset = re.query(total, code);
    let (value, lo, hi) = model.find_value(offset);
    debug!("\tDecoding value {} of offset {} with total {}", value, offset, total);
    let mut shift_bytes = 0u;
    re.process(total, lo, hi, |_| shift_bytes+=1);
    (value, shift_bytes)
}


/// An arithmetic encoder helper
pub struct Encoder<W> {
    stream: W,
    range: RangeEncoder,
    buffer: Vec<Symbol>,
}

impl<W: Writer> Encoder<W> {
    /// Create a new encoder on top of a given Writer
    pub fn new(w: W) -> Encoder<W> {
        Encoder {
            stream: w,
            range: RangeEncoder::new(RANGE_DEFAULT_THRESHOLD),
            buffer: Vec::with_capacity(4),
        }
    }

    /// Encode an abstract value under the given Model
    pub fn encode<V: Copy + Show, M: Model<V>>(&mut self, value: V, model: &M) -> IoResult<()> {
        self.buffer.truncate(0);
        encode(value, model, &mut self.range, &mut self.buffer);
        self.stream.write(self.buffer.as_slice())
    }

    /// Finish encoding by writing the code tail word
    pub fn finish(mut self) -> (W, IoResult<()>) {
        assert!(BORDER_BITS == 32);
        let code = self.range.get_code_tail();
        let result = self.stream.write_be_u32(code);
        let result = result.and(self.stream.flush());
        (self.stream, result)
    }

    /// Flush the output stream
    pub fn flush(&mut self) -> IoResult<()> {
        self.stream.flush()
    }

    /// Return the number of bytes lost due to threshold cuts and integer operations
    #[cfg(tune)]
    pub fn get_bytes_lost(&self) -> (f32, f32) {
        let (a,b) = self.range.get_bits_lost();
        (a/8.0, b/8.0)
    }
}

/// An arithmetic decoder helper
pub struct Decoder<R> {
    stream: R,
    range: RangeEncoder,
    code: Border,
    bytes_pending: uint,
}

impl<R: Reader> Decoder<R> {
    /// Create a decoder on top of a given Reader
    pub fn new(r: R) -> Decoder<R> {
        Decoder {
            stream: r,
            range: RangeEncoder::new(RANGE_DEFAULT_THRESHOLD),
            code: 0,
            bytes_pending: BORDER_BITS>>3,
        }
    }

    fn feed(&mut self) -> IoResult<()> {
        while self.bytes_pending != 0 {
            let b = try!(self.stream.read_u8());
            self.code = (self.code<<8) + (b as Border);
            self.bytes_pending -= 1;
        }
        Ok(())
    }

    /// Decode an abstract value based on the given Model
    pub fn decode<V: Copy + Show, M: Model<V>>(&mut self, model: &M) -> IoResult<V> {
        self.feed().unwrap();
        let (value, shift) = decode(self.code, model, &mut self.range);
        self.bytes_pending = shift;
        Ok(value)
    }

    /// Finish decoding
    pub fn finish(mut self) -> (R, IoResult<()>)  {
        let err = self.feed();
        (self.stream, err)
    }
}
