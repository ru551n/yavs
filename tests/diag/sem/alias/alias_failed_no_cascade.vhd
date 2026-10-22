-- yavs-test: std=2008
-- expect: E1610 13:16 the aliased name is not a static name of an object
-- LRM 6.6.2, 8.1, 8.2: one diagnostic for a failed alias; its attribute and overloaded call uses report nothing
entity e is
end entity;
architecture a of e is
  signal s : bit_vector(15 downto 0);
  signal i : natural;
  procedure p(x : bit_vector) is begin end procedure;
  procedure p(x : integer) is begin end procedure;
  constant n : natural := 3;
  alias fine is s(n downto 0);
  alias bad is s(i downto 0);
begin
  assert bad'length = 8 and fine'length = 4;
  process
  begin
    p(bad);
    p(bad(0 downto 0));
    wait;
  end process;
end architecture;
