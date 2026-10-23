-- yavs-test: std=2008
-- expect: E1621 13:14 this prefix has no value for attribute `a`
-- LRM 7.2, 4.3.3 (an alias of an element is another named entity than the object)
entity e is
  port (v : in bit_vector(1 downto 0));
  attribute a : integer;
  attribute a of v : signal is 1;
  alias b : bit is v(0);
end entity;
architecture x of e is
begin
  process begin
    assert b'a = 1;
    wait;
  end process;
end architecture;
