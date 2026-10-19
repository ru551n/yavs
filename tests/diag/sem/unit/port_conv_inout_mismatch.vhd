-- yavs-test: std=2008
-- expect: E1301 18:37 the formal part does not give the actual's type
-- help: a function of one parameter of the formal's type
-- LRM 6.5.7.1
-- The formal part of an `inout` port association converts the formal to the actual's
-- type: `to_bv` gives BIT_VECTOR, the actual is a STD_ULOGIC_VECTOR.
library ieee; use ieee.std_logic_1164.all;
entity sub is
  port (b : inout integer);
end entity;
architecture a of sub is begin b <= 5; end architecture;
library ieee; use ieee.std_logic_1164.all;
entity top is end entity;
architecture a of top is
  function to_bv(i : integer) return bit_vector is begin return "0101"; end function;
  function to_int(v : std_logic_vector) return integer is begin return 5; end function;
  signal t : std_logic_vector(3 downto 0);
begin u : entity work.sub port map (to_bv(b) => to_int(t));
end architecture;
