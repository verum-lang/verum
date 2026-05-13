# `core.text.numeric.modular` — audit

> Status: **partial**. 9 number-theoretic functions over BigInt: gcd, lcm,
> ext_gcd, mod_pow, mod_inverse, mod_sqrt, is_probable_prime, crt, crt2.
> All currently inherit the BigInt defect classes (Iterator dispatch,
> function-id collision, Int operator dispatch).

## Defect classes (inherited)
Same as `core/text/numeric/bigint`.

## Action items
- 8 unit tests pinning the canonical small cases:
  gcd(7,7)=7, gcd(12,18)=6, gcd(0,0)=0, lcm(4,6)=12,
  mod_pow(2,10,1000)=24, mod_pow(1,N,M)=1,
  mod_inverse(3,7)=5, mod_inverse(2,4)=Err.
- Once BigInt arithmetic stabilises, expand to property tests:
  - gcd is commutative, associative, divides both args
  - lcm * gcd = a * b
  - ext_gcd: a*x + b*y == gcd(a,b)
  - mod_pow: (a^b)^c == a^(b*c) mod m
  - is_probable_prime: deterministic on Miller-Rabin witness sets
