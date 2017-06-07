//            DO WHAT THE FUCK YOU WANT TO PUBLIC LICENSE
//                    Version 2, December 2004
//
// Copyleft (ↄ) meh. <meh@schizofreni.co> | http://meh.schizofreni.co
//
// Everyone is permitted to copy and distribute verbatim or modified
// copies of this license document, and changing it is allowed as long
// as the name is changed.
//
//            DO WHAT THE FUCK YOU WANT TO PUBLIC LICENSE
//   TERMS AND CONDITIONS FOR COPYING, DISTRIBUTION AND MODIFICATION
//
//  0. You just DO WHAT THE FUCK YOU WANT TO.

use std::io::Cursor;
use std::net::Ipv4Addr;
use byteorder::{WriteBytesExt, BigEndian};

use error::*;
use buffer::{self, Buffer};
use size::Min;
use builder::{Builder as Build, Finalization};
use ip::Protocol;
use ip::v4::Packet;
use ip::v4::Flags;
use ip::v4::option;
use ip::v4::checksum;
use icmp;

pub struct Builder<B: Buffer = buffer::Dynamic> {
	buffer:    B,
	finalizer: Finalization,

	options: bool,
	payload: bool,
}

impl<B: Buffer> Build<B> for Builder<B> {
	fn with(mut buffer: B) -> Result<Self> {
		// Allocate size enough to contain the minimum size of an IPv4 packet.
		buffer.next(Packet::<()>::min())?;

		// Set the version to 4 and header length to 5.
		buffer.data_mut()[0] = (4 << 4) | 5;

		Ok(Builder {
			buffer:    buffer,
			finalizer: Default::default(),

			options: false,
			payload: false,
		})
	}

	fn finalizer(&mut self) -> &mut Finalization {
		&mut self.finalizer
	}

	fn build(self) -> Result<B::Inner> {
		let mut buffer = self.buffer.into_inner();
		self.finalizer.finalize(buffer.as_mut())?;
		Ok(buffer)
	}
}

impl Default for Builder<buffer::Dynamic> {
	fn default() -> Self {
		Builder::with(buffer::Dynamic::default()).unwrap()
	}
}

impl<B: Buffer> Builder<B> {
	pub fn dscp(mut self, value: u8) -> Result<Self> {
		let old = self.buffer.data()[1];
		self.buffer.data_mut()[1] = (old & 0b11) | (value & 0b1111_111) << 2;

		Ok(self)
	}

	pub fn ecn(mut self, value: u8) -> Result<Self> {
		let old = self.buffer.data()[1];
		self.buffer.data_mut()[1] = (old & 0b1111_11) | (value & 0b11);

		Ok(self)
	}

	pub fn id(mut self, value: u16) -> Result<Self> {
		Cursor::new(&mut self.buffer.data_mut()[4 ..])
			.write_u16::<BigEndian>(value)?;

		Ok(self)
	}

	pub fn flags(mut self, value: Flags) -> Result<Self> {
		Cursor::new(&mut self.buffer.data_mut()[6 ..])
			.write_u16::<BigEndian>(value.bits())?;

		Ok(self)
	}

	pub fn offset(mut self, value: u16) -> Result<Self> {
		Cursor::new(&mut self.buffer.data_mut()[6 ..])
			.write_u16::<BigEndian>(value)?;

		Ok(self)
	}

	pub fn ttl(mut self, value: u8) -> Result<Self> {
		self.buffer.data_mut()[8] = value;

		Ok(self)
	}

	pub fn source(mut self, value: Ipv4Addr) -> Result<Self> {
		let ip = value.octets();

		self.buffer.data_mut()[12] = ip[0];
		self.buffer.data_mut()[13] = ip[1];
		self.buffer.data_mut()[14] = ip[2];
		self.buffer.data_mut()[15] = ip[3];

		Ok(self)
	}

	pub fn destination(mut self, value: Ipv4Addr) -> Result<Self> {
		let ip = value.octets();

		self.buffer.data_mut()[16] = ip[0];
		self.buffer.data_mut()[17] = ip[1];
		self.buffer.data_mut()[18] = ip[2];
		self.buffer.data_mut()[19] = ip[3];

		Ok(self)
	}

	pub fn protocol(mut self, value: Protocol) -> Result<Self> {
		self.buffer.data_mut()[9] = value.into();

		Ok(self)
	}

	pub fn payload<'a, T: IntoIterator<Item = &'a u8>>(mut self, value: T) -> Result<Self> {
		if self.payload {
			return Err(ErrorKind::InvalidPacket.into());
		}

		self.payload = true;

		for byte in value.into_iter() {
			self.buffer.more(1)?;
			*self.buffer.data_mut().last_mut().unwrap() = *byte;
		}

		Ok(self)
	}

	pub fn icmp(mut self) -> Result<icmp::Builder<B>> {
		if self.payload {
			return Err(ErrorKind::InvalidPacket.into());
		}

		self = self.protocol(Protocol::Icmp)?;

		let offset = self.buffer.offset();
		let length = self.buffer.length();

		let mut icmp = icmp::Builder::with(self.buffer)?;
		icmp.finalizer().extend(self.finalizer.into());
		icmp.finalizer().add(move |out| {
			// Get the length of the header.
			let header = out[offset] & 0b1111;

			// Calculate and wite the total length of the packet.
			let length = length + (out.len() - (offset + length));
			Cursor::new(&mut out[offset + 2 ..])
				.write_u16::<BigEndian>(length as u16)?;

			// Calculate and write the checksum.
			let checksum = checksum(&out[offset .. offset + (header as usize * 4)]);
			Cursor::new(&mut out[offset + 10 ..])
				.write_u16::<BigEndian>(checksum)?;

			Ok(())
		});

		Ok(icmp)
	}
}

#[cfg(test)]
mod test {
	use std::net::Ipv4Addr;
	use builder::Builder;
	use packet::Packet;
	use ip;

	#[test]
	fn icmp() {
		let packet = ip::v4::Builder::default()
			.id(0x2d87).unwrap()
			.ttl(64).unwrap()
			.source("66.102.1.108".parse().unwrap()).unwrap()
			.destination("192.168.0.79".parse().unwrap()).unwrap()
			.icmp().unwrap()
				.echo().unwrap().request().unwrap()
					.identifier(42).unwrap()
					.sequence(2).unwrap()
					.payload(b"test").unwrap()
					.build().unwrap();

		let packet = ip::v4::Packet::new(&packet).unwrap();
		
		assert_eq!(packet.id(), 0x2d87);
		assert!(packet.flags().is_empty());
		assert_eq!(packet.length(), 32);
		assert_eq!(packet.ttl(), 64);
		assert_eq!(packet.protocol(), ip::Protocol::Icmp);
		assert_eq!(packet.source(), "66.102.1.108".parse::<Ipv4Addr>().unwrap());
		assert_eq!(packet.destination(), "192.168.0.79".parse::<Ipv4Addr>().unwrap());
		assert!(packet.is_valid());
	}
}