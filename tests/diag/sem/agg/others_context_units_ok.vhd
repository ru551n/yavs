-- yavs-test: std=2008
-- expect: none
-- LRM 9.3.3.3
-- `others` aggregates as generic and port actuals and defaults, record element targets,
-- conditional signal assignments and slice signal targets.
entity sub is
  generic (g : bit_vector(3 downto 0) := (others => '0'));
  port (i : in bit_vector(3 downto 0) := (others => '0'); o : out bit_vector(3 downto 0));
end entity;
architecture a of sub is
begin
  o <= g;
end architecture;
entity top is end entity;
architecture a of top is
  type rec is record f : bit_vector(1 to 3); end record;
  signal s, s2, s3 : bit_vector(3 downto 0);
  signal sr : rec;
begin
  u : entity work.sub generic map (g => (others => '1')) port map (i => (others => '1'), o => s);
  sr.f <= (others => '1');
  s2 <= (others => '0') when sr.f(1) = '1' else (others => '1');
  process
    variable r : rec;
  begin
    r.f := (others => '0');
    s3(1 downto 0) <= (others => '1');
    wait;
  end process;
end architecture;
