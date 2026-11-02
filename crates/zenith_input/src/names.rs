//! Reading and writing the names of buttons and axes.

/// Name without case, spaces, `_` and `-`: `Left_Shift`, `leftshift` and
/// `LeftShift` are the same name.
pub fn normalize(name: &str) -> String {
	name.chars()
		.filter(char::is_ascii_alphanumeric)
		.map(|c| c.to_ascii_lowercase())
		.collect()
}

/// A plain enum that has a name for every variant: `ALL`, `name`, `Display`,
/// `FromStr` (any case, `_` and `-` allowed) and serde as a string.
macro_rules! string_enum {
	($(#[$meta:meta])* $vis:vis enum $type:ident { $($(#[$vmeta:meta])* $variant:ident,)* }) => {
		$(#[$meta])*
		#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
		$vis enum $type {
			$($(#[$vmeta])* $variant,)*
		}

		impl $type {
			/// Every variant, in the order of the declaration.
			pub const ALL: &'static [Self] = &[$(Self::$variant,)*];

			/// The name of the variant.
			#[must_use]
			pub const fn name(self) -> &'static str {
				match self {
					$(Self::$variant => stringify!($variant),)*
				}
			}
		}

		impl ::std::fmt::Display for $type {
			fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
				f.write_str(self.name())
			}
		}

		impl ::std::str::FromStr for $type {
			type Err = ::zenith_error::ZenithError;

			fn from_str(text: &str) -> Result<Self, Self::Err> {
				let wanted = $crate::names::normalize(text);
				Self::ALL
					.iter()
					.copied()
					.find(|item| $crate::names::normalize(item.name()) == wanted)
					.ok_or_else(|| {
						::zenith_error::ZenithError::InvalidSettings(format!(
							"unknown {} {text:?}",
							stringify!($type)
						))
					})
			}
		}

		impl ::serde::Serialize for $type {
			fn serialize<S: ::serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
				serializer.serialize_str(self.name())
			}
		}

		impl<'de> ::serde::Deserialize<'de> for $type {
			fn deserialize<D: ::serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
				let text = <String as ::serde::Deserialize>::deserialize(deserializer)?;
				text.parse().map_err(::serde::de::Error::custom)
			}
		}
	};
}

pub(crate) use string_enum;
