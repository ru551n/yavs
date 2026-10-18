-- yavs-test: std=2008
-- expect: none
-- LRM 11.3
entity e is end entity;
architecture a of e is
  signal s : bit;
  procedure pr (signal x : out bit) is
  begin
    x <= '1';
  end procedure;
begin
  process begin
    pr(s);
    wait;
  end process;
end architecture;
