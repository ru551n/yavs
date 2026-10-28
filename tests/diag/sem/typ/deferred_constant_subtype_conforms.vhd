-- yavs-test: std=2008
-- expect: E1227 14:16 the full declaration of `X` has subtype `INTEGER`, the deferred constant `REAL`
-- expect: E1227 15:16 the full declaration of `Y` has subtype `S2`, the deferred constant `S1`
-- expect: E1227 16:16 the full declaration of `V` has subtype `BIT_VECTOR`, the deferred constant `BIT_VECTOR`
-- LRM 6.4.2.2: the subtype indications of the deferred and the full constant declaration conform (#809)
package p is
  subtype s1 is integer;
  subtype s2 is integer;
  constant X : real;
  constant Y : s1;
  constant V, W : bit_vector(0 to 3);
end package;
package body p is
  constant X : integer := 1;
  constant Y : s2 := 0;
  constant V : bit_vector(3 downto 0) := "0000";
  constant W : bit_vector(0 to 3) := "0000";
end package body;
