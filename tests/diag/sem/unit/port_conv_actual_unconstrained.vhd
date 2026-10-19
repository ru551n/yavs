-- yavs-test: std=2008
-- expect: E1312 13:43 the conversion of the actual of unconstrained formal `i` must give its bounds
-- expect: E1312 14:43 the conversion of the actual of unconstrained formal `i` must give its bounds
-- LRM 5.3.2.2: the type mark or the result subtype must define the formal's index range
library ieee; use ieee.numeric_bit.all;
entity sub is port (i : in bit_vector); end entity;
architecture r of sub is begin end architecture;
library ieee; use ieee.numeric_bit.all;
entity e is end entity;
architecture a of e is
  function f(x : unsigned) return bit_vector is begin return bit_vector(x); end function;
  signal u : unsigned(3 downto 0);
begin u1 : entity work.sub port map (i => bit_vector(u));
      u2 : entity work.sub port map (i => f(u));
end architecture;
