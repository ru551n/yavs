-- yavs-test: std=2019
-- expect: E1300 11:29 a range record call of another type than the index
-- LRM 5.3.2.2: the type of each discrete range is the type of its index
entity e is end entity;
architecture a of e is
  subtype cr is character'range'record;
  function c(k : integer) return cr is begin return (left => 'a', right => 'b', direction => ascending); end function;
begin
  process
    variable v : bit_vector(0 to 3);
    variable w : bit_vector(c(1));
  begin
    wait;
  end process;
end architecture;
