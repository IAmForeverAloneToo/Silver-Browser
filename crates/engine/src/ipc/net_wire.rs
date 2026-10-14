//! The network's values on the wire (M34.2): a request as a renderer
//! asks for it, a response as the browser answers, and what they carry.
//! A request's abort handle does not go: the renderer sends a
//! cancellation instead. A policy list goes as its policies' text, parsed
//! again by the side that receives it.

use std::net::{Ipv4Addr, Ipv6Addr};

use net::csp::{Csp, CspParts, Disposition, Violation};
use net::{
    AbortHandle, AddressSpace, CacheMode, Credentials, Destination, HeaderList, Host, Initiator,
    LocalNetworkAccess, Method, MimeType, Mode, NetworkError, Origin, Partition, Redirect,
    Referrer, ReferrerPolicy, Request, Response, ResponseType, TlsFailure, Url,
};

use super::codec::{Decoder, Encoder, Malformed, Wire, wire_unit_enum};

impl Wire for net::cookies::Context {
    fn encode(&self, out: &mut Encoder) {
        out.bool(self.same_site);
        out.bool(self.top_level);
        out.bool(self.safe_method);
        self.partition.encode(out);
    }
    fn decode(input: &mut Decoder<'_>) -> Result<Self, Malformed> {
        Ok(net::cookies::Context {
            same_site: input.bool()?,
            top_level: input.bool()?,
            safe_method: input.bool()?,
            partition: Option::decode(input)?,
        })
    }
}

wire_unit_enum!(Destination {
    0 => Destination::Document,
    1 => Destination::Iframe,
    2 => Destination::Style,
    3 => Destination::Image,
    4 => Destination::Script,
    5 => Destination::Worker,
    6 => Destination::Empty,
    7 => Destination::Report,
});

wire_unit_enum!(Mode {
    0 => Mode::Navigate,
    1 => Mode::SameOrigin,
    2 => Mode::NoCors,
    3 => Mode::Cors,
});

wire_unit_enum!(Credentials {
    0 => Credentials::Omit,
    1 => Credentials::SameOrigin,
    2 => Credentials::Include,
});

wire_unit_enum!(Redirect {
    0 => Redirect::Follow,
    1 => Redirect::Error,
    2 => Redirect::Manual,
});

wire_unit_enum!(ReferrerPolicy {
    0 => ReferrerPolicy::NoReferrer,
    1 => ReferrerPolicy::NoReferrerWhenDowngrade,
    2 => ReferrerPolicy::SameOrigin,
    3 => ReferrerPolicy::Origin,
    4 => ReferrerPolicy::StrictOrigin,
    5 => ReferrerPolicy::OriginWhenCrossOrigin,
    6 => ReferrerPolicy::StrictOriginWhenCrossOrigin,
    7 => ReferrerPolicy::UnsafeUrl,
});

wire_unit_enum!(CacheMode {
    0 => CacheMode::Default,
    1 => CacheMode::NoStore,
    2 => CacheMode::Reload,
    3 => CacheMode::NoCache,
    4 => CacheMode::ForceCache,
    5 => CacheMode::OnlyIfCached,
});

wire_unit_enum!(AddressSpace {
    0 => AddressSpace::Loopback,
    1 => AddressSpace::Local,
    2 => AddressSpace::Public,
});

wire_unit_enum!(ResponseType {
    0 => ResponseType::Basic,
    1 => ResponseType::Cors,
    2 => ResponseType::Opaque,
    3 => ResponseType::OpaqueRedirect,
});

wire_unit_enum!(Disposition {
    0 => Disposition::Enforce,
    1 => Disposition::Report,
});

wire_unit_enum!(net::http::Malformed {
    0 => net::http::Malformed::StatusLine,
    1 => net::http::Malformed::FieldLine,
    2 => net::http::Malformed::ContentLength,
    3 => net::http::Malformed::TransferEncoding,
    4 => net::http::Malformed::Chunk,
});

impl Wire for Method {
    fn encode(&self, out: &mut Encoder) {
        out.str(self.as_str());
    }
    fn decode(input: &mut Decoder<'_>) -> Result<Self, Malformed> {
        Method::parse(input.str()?).ok_or(Malformed::Invalid("method"))
    }
}

impl Wire for HeaderList {
    fn encode(&self, out: &mut Encoder) {
        out.length(self.len());
        for (name, value) in self.iter() {
            out.bytes(name);
            out.bytes(value);
        }
    }
    fn decode(input: &mut Decoder<'_>) -> Result<Self, Malformed> {
        let mut headers = HeaderList::new();
        for _ in 0..input.count()? {
            let name = input.bytes()?;
            let value = input.bytes()?;
            headers.append(name, value);
        }
        Ok(headers)
    }
}

impl Wire for Host {
    fn encode(&self, out: &mut Encoder) {
        match self {
            Host::Domain(name) => {
                out.u8(0);
                out.str(name);
            }
            Host::Ipv4(address) => {
                out.u8(1);
                out.u32(u32::from(*address));
            }
            Host::Ipv6(address) => {
                out.u8(2);
                for segment in address.segments() {
                    out.u16(segment);
                }
            }
            Host::Opaque(name) => {
                out.u8(3);
                out.str(name);
            }
            Host::Empty => out.u8(4),
        }
    }
    fn decode(input: &mut Decoder<'_>) -> Result<Self, Malformed> {
        Ok(match input.u8()? {
            0 => Host::Domain(input.str()?.to_owned()),
            1 => Host::Ipv4(Ipv4Addr::from(input.u32()?)),
            2 => {
                let mut segments = [0u16; 8];
                for segment in &mut segments {
                    *segment = input.u16()?;
                }
                Host::Ipv6(Ipv6Addr::from(segments))
            }
            3 => Host::Opaque(input.str()?.to_owned()),
            4 => Host::Empty,
            tag => return Err(Malformed::Tag("Host", u32::from(tag))),
        })
    }
}

impl Wire for Origin {
    fn encode(&self, out: &mut Encoder) {
        match self {
            Origin::Tuple { scheme, host, port } => {
                out.u8(0);
                out.str(scheme);
                host.encode(out);
                port.encode(out);
            }
            Origin::Opaque(number) => {
                out.u8(1);
                out.u64(*number);
            }
        }
    }
    fn decode(input: &mut Decoder<'_>) -> Result<Self, Malformed> {
        Ok(match input.u8()? {
            0 => Origin::Tuple {
                scheme: input.str()?.to_owned(),
                host: Host::decode(input)?,
                port: Option::decode(input)?,
            },
            1 => Origin::Opaque(input.u64()?),
            tag => return Err(Malformed::Tag("Origin", u32::from(tag))),
        })
    }
}

impl Wire for Initiator {
    fn encode(&self, out: &mut Encoder) {
        self.url.encode(out);
        self.address_space.encode(out);
        self.origin.encode(out);
    }
    fn decode(input: &mut Decoder<'_>) -> Result<Self, Malformed> {
        Ok(Initiator {
            url: Url::decode(input)?,
            address_space: AddressSpace::decode(input)?,
            origin: Origin::decode(input)?,
        })
    }
}

impl Wire for LocalNetworkAccess {
    fn encode(&self, out: &mut Encoder) {
        self.origin.encode(out);
        out.bool(self.local);
        out.bool(self.loopback);
    }
    fn decode(input: &mut Decoder<'_>) -> Result<Self, Malformed> {
        Ok(LocalNetworkAccess {
            origin: Origin::decode(input)?,
            local: input.bool()?,
            loopback: input.bool()?,
        })
    }
}

impl Wire for Partition {
    fn encode(&self, out: &mut Encoder) {
        self.top.encode(out);
        out.bool(self.cross_site);
    }
    fn decode(input: &mut Decoder<'_>) -> Result<Self, Malformed> {
        Ok(Partition {
            top: Origin::decode(input)?,
            cross_site: input.bool()?,
        })
    }
}

impl Wire for Referrer {
    fn encode(&self, out: &mut Encoder) {
        match self {
            Referrer::None => out.u8(0),
            Referrer::Client => out.u8(1),
            Referrer::Url(url) => {
                out.u8(2);
                url.encode(out);
            }
        }
    }
    fn decode(input: &mut Decoder<'_>) -> Result<Self, Malformed> {
        Ok(match input.u8()? {
            0 => Referrer::None,
            1 => Referrer::Client,
            2 => Referrer::Url(Box::new(Url::decode(input)?)),
            tag => return Err(Malformed::Tag("Referrer", u32::from(tag))),
        })
    }
}

impl Wire for MimeType {
    fn encode(&self, out: &mut Encoder) {
        out.bytes(&self.to_bytes());
    }
    fn decode(input: &mut Decoder<'_>) -> Result<Self, Malformed> {
        MimeType::parse_bytes(input.bytes()?).ok_or(Malformed::Invalid("MIME type"))
    }
}

impl Wire for Csp {
    fn encode(&self, out: &mut Encoder) {
        let parts = self.parts();
        out.length(parts.policies.len());
        for (serialized, disposition, from_meta) in &parts.policies {
            out.str(serialized);
            disposition.encode(out);
            out.bool(*from_meta);
        }
        parts.self_origin.encode(out);
        out.bool(parts.overflowed);
    }
    fn decode(input: &mut Decoder<'_>) -> Result<Self, Malformed> {
        let mut policies = Vec::new();
        for _ in 0..input.count()? {
            policies.push((
                input.str()?.to_owned(),
                Disposition::decode(input)?,
                input.bool()?,
            ));
        }
        Ok(Csp::from_parts(CspParts {
            policies,
            self_origin: Origin::decode(input)?,
            overflowed: input.bool()?,
        }))
    }
}

impl Wire for Violation {
    fn encode(&self, out: &mut Encoder) {
        out.str(&self.directive);
        out.str(&self.value);
        out.str(&self.effective);
        out.str(&self.blocked);
        out.str(&self.what);
        self.disposition.encode(out);
        out.str(&self.policy);
        out.str(&self.sample);
        self.report_uri.encode(out);
    }
    fn decode(input: &mut Decoder<'_>) -> Result<Self, Malformed> {
        Ok(Violation {
            directive: String::decode(input)?,
            value: String::decode(input)?,
            effective: String::decode(input)?,
            blocked: String::decode(input)?,
            what: String::decode(input)?,
            disposition: Disposition::decode(input)?,
            policy: String::decode(input)?,
            sample: String::decode(input)?,
            report_uri: Vec::decode(input)?,
        })
    }
}

impl Wire for Request {
    fn encode(&self, out: &mut Encoder) {
        self.url.encode(out);
        self.method.encode(out);
        self.headers.encode(out);
        match &self.body {
            None => out.u8(0),
            Some(body) => {
                out.u8(1);
                out.bytes(body);
            }
        }
        self.destination.encode(out);
        self.mode.encode(out);
        self.credentials.encode(out);
        self.redirect.encode(out);
        self.origin.encode(out);
        out.bool(self.user_activated);
        out.bool(self.preflight);
        self.initiator.encode(out);
        self.body_limit.encode(out);
        self.csp.encode(out);
        out.str(&self.nonce);
        out.bool(self.parser_inserted);
        out.bool(self.form_submission);
        self.referrer.encode(out);
        self.referrer_policy.encode(out);
        self.cache.encode(out);
        self.partition.encode(out);
        self.local_network.encode(out);
        self.target_address_space.encode(out);
    }
    fn decode(input: &mut Decoder<'_>) -> Result<Self, Malformed> {
        Ok(Request {
            url: Url::decode(input)?,
            method: Method::decode(input)?,
            headers: HeaderList::decode(input)?,
            body: match input.u8()? {
                0 => None,
                1 => Some(input.bytes()?.to_vec()),
                tag => return Err(Malformed::Tag("body", u32::from(tag))),
            },
            destination: Destination::decode(input)?,
            mode: Mode::decode(input)?,
            credentials: Credentials::decode(input)?,
            redirect: Redirect::decode(input)?,
            origin: Option::decode(input)?,
            user_activated: input.bool()?,
            preflight: input.bool()?,
            initiator: Option::decode(input)?,
            abort: AbortHandle::new(),
            body_limit: Option::decode(input)?,
            csp: Option::decode(input)?,
            nonce: String::decode(input)?,
            parser_inserted: input.bool()?,
            form_submission: input.bool()?,
            referrer: Referrer::decode(input)?,
            referrer_policy: Option::decode(input)?,
            cache: CacheMode::decode(input)?,
            partition: Option::decode(input)?,
            local_network: Option::decode(input)?,
            target_address_space: Option::decode(input)?,
        })
    }
}

/// A response without its body, which goes in pieces.
pub struct Head<'a>(pub &'a Response);

impl Wire for Response {
    fn encode(&self, out: &mut Encoder) {
        Head(self).encode(out);
        out.bytes(&self.body);
    }
    fn decode(input: &mut Decoder<'_>) -> Result<Self, Malformed> {
        let mut response = decode_head(input)?;
        response.body = input.bytes()?.to_vec();
        Ok(response)
    }
}

impl Head<'_> {
    pub fn encode(&self, out: &mut Encoder) {
        let response = self.0;
        response.url_list.encode(out);
        out.u16(response.status);
        out.bytes(&response.status_message);
        response.headers.encode(out);
        response.mime_type.encode(out);
        response.address_space.encode(out);
        response.response_type.encode(out);
        out.length(response.cors_exposed.len());
        for name in &response.cors_exposed {
            out.bytes(name);
        }
    }
}

/// A response's head, its body empty.
pub fn decode_head(input: &mut Decoder<'_>) -> Result<Response, Malformed> {
    let url_list = Vec::<Url>::decode(input)?;
    if url_list.is_empty() {
        return Err(Malformed::Invalid("URL list"));
    }
    Ok(Response {
        url_list,
        status: input.u16()?,
        status_message: input.bytes()?.to_vec(),
        headers: HeaderList::decode(input)?,
        body: Vec::new(),
        mime_type: Option::decode(input)?,
        address_space: Option::decode(input)?,
        response_type: ResponseType::decode(input)?,
        cors_exposed: {
            let mut names = Vec::new();
            for _ in 0..input.count()? {
                names.push(input.bytes()?.to_vec());
            }
            names
        },
    })
}

impl Wire for TlsFailure {
    fn encode(&self, out: &mut Encoder) {
        match self {
            TlsFailure::Expired => out.u8(0),
            TlsFailure::NotYetValid => out.u8(1),
            TlsFailure::UnknownIssuer => out.u8(2),
            TlsFailure::WrongName => out.u8(3),
            TlsFailure::Other(what) => {
                out.u8(4);
                out.str(what);
            }
        }
    }
    fn decode(input: &mut Decoder<'_>) -> Result<Self, Malformed> {
        Ok(match input.u8()? {
            0 => TlsFailure::Expired,
            1 => TlsFailure::NotYetValid,
            2 => TlsFailure::UnknownIssuer,
            3 => TlsFailure::WrongName,
            4 => TlsFailure::Other(input.str()?.to_owned()),
            tag => return Err(Malformed::Tag("TlsFailure", u32::from(tag))),
        })
    }
}

impl Wire for NetworkError {
    fn encode(&self, out: &mut Encoder) {
        let text = |out: &mut Encoder, tag: u8, text: &str| {
            out.u8(tag);
            out.str(text);
        };
        match self {
            NetworkError::BadScheme(scheme) => text(out, 0, scheme),
            NetworkError::BadPort(port) => {
                out.u8(1);
                out.u16(*port);
            }
            NetworkError::AddressSpace { initiator, target } => {
                out.u8(2);
                initiator.encode(out);
                target.encode(out);
            }
            NetworkError::TargetAddressSpace { named, reached } => {
                out.u8(3);
                named.encode(out);
                reached.encode(out);
            }
            NetworkError::MixedContent => out.u8(4),
            NetworkError::Resolve(what) => text(out, 5, what),
            NetworkError::Connect(what) => text(out, 6, what),
            NetworkError::Tls(failure) => {
                out.u8(7);
                failure.encode(out);
            }
            NetworkError::Malformed(malformed) => {
                out.u8(8);
                malformed.encode(out);
            }
            NetworkError::UnaskedUpgrade => out.u8(9),
            NetworkError::Truncated => out.u8(10),
            NetworkError::Io(what) => text(out, 11, what),
            NetworkError::HeadersTooLarge => out.u8(12),
            NetworkError::BodyTooLarge => out.u8(13),
            NetworkError::ContentCoding => out.u8(14),
            NetworkError::DataUrl => out.u8(15),
            NetworkError::File(what) => text(out, 16, what),
            NetworkError::Timeout => out.u8(17),
            NetworkError::RedirectLimit => out.u8(18),
            NetworkError::BadRedirect => out.u8(19),
            NetworkError::Aborted => out.u8(20),
            NetworkError::Cors(what) => text(out, 21, what),
            NetworkError::CrossOriginResourcePolicy => out.u8(22),
            NetworkError::Redirected => out.u8(23),
            NetworkError::Refused(what) => text(out, 24, what),
            NetworkError::Csp(what) => text(out, 25, what),
            NetworkError::NotCached => out.u8(26),
        }
    }
    fn decode(input: &mut Decoder<'_>) -> Result<Self, Malformed> {
        let tag = input.u8()?;
        let text = |input: &mut Decoder<'_>| input.str().map(str::to_owned);
        Ok(match tag {
            0 => NetworkError::BadScheme(text(input)?),
            1 => NetworkError::BadPort(input.u16()?),
            2 => NetworkError::AddressSpace {
                initiator: AddressSpace::decode(input)?,
                target: AddressSpace::decode(input)?,
            },
            3 => NetworkError::TargetAddressSpace {
                named: AddressSpace::decode(input)?,
                reached: AddressSpace::decode(input)?,
            },
            4 => NetworkError::MixedContent,
            5 => NetworkError::Resolve(text(input)?),
            6 => NetworkError::Connect(text(input)?),
            7 => NetworkError::Tls(TlsFailure::decode(input)?),
            8 => NetworkError::Malformed(net::http::Malformed::decode(input)?),
            9 => NetworkError::UnaskedUpgrade,
            10 => NetworkError::Truncated,
            11 => NetworkError::Io(text(input)?),
            12 => NetworkError::HeadersTooLarge,
            13 => NetworkError::BodyTooLarge,
            14 => NetworkError::ContentCoding,
            15 => NetworkError::DataUrl,
            16 => NetworkError::File(text(input)?),
            17 => NetworkError::Timeout,
            18 => NetworkError::RedirectLimit,
            19 => NetworkError::BadRedirect,
            20 => NetworkError::Aborted,
            21 => NetworkError::Cors(text(input)?),
            22 => NetworkError::CrossOriginResourcePolicy,
            23 => NetworkError::Redirected,
            24 => NetworkError::Refused(text(input)?),
            25 => NetworkError::Csp(text(input)?),
            26 => NetworkError::NotCached,
            tag => return Err(Malformed::Tag("NetworkError", u32::from(tag))),
        })
    }
}
