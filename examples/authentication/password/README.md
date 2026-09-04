# Password login plugin

Cowboy's local password method is a host plugin. The Controller still owns the
`users` password hash and session cookies; this package owns the login slot.

No plugin-owned tables. Identity stays in the product user plane.
