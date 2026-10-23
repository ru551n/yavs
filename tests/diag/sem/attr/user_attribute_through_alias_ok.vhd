-- yavs-test: std=2008
-- expect: none
-- LRM 7.2, 4.3.3 (an attribute of an object applies to every alias of the whole object; #642;
-- the second reference simulator rejects)
entity e is
  port (s : in integer);
  attribute a : integer;
  attribute a of s : signal is 1;
  alias s1 : integer is s;
  alias s2 : integer is s1;
end entity;
architecture x of e is
begin
  process begin
    assert s1'a = s'a and s2'a = 1;
    wait;
  end process;
end architecture;
