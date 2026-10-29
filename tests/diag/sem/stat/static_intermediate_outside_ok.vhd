-- yavs-test: std=2008
-- expect: none
-- LRM 9.4.1, 5.2.3.1 (#872: a static value keeps its exact value up to the outermost operation)
entity e is
  port (cnt : out natural range 0 to 2 ** 31 - 1 := 0);
end entity;
architecture a of e is
  type t is array (0 to 2 ** 33 / 2 ** 30) of bit;
  signal s : natural range 0 to 2 ** 31 - 1;
  constant lo : integer := -2 ** 31;
  constant k : integer := 2;
  constant hi : integer := k ** 31 - 1;
begin
  process
    variable v : natural := 0;
  begin
    if v < 2 ** 31 - 1 then
      v := 1;
    end if;
    wait;
  end process;
end architecture;
