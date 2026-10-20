-- difftest: top=tb_port_actual_generic_bounds std=2008
-- The actual of a port whose subtype names the instance's generic takes its bounds from that subtype
-- (here a null range), as a generic map actual already does.
package pk_pag is
  type rec is record x : integer; y : integer; z : boolean; end record;
end package;
use work.pk_pag.all;
entity sub_pag is
  generic (r : rec);
  port (i : in bit_vector(1 to r.y); o : out bit);
end entity;
architecture test of sub_pag is begin
  o <= '1' when r.z else '0';
  process begin
    wait for 1 ns + r.y * 1 ns;
    report "len=" & integer'image(i'length) & " o=" & bit'image(o);
    wait;
  end process;
end architecture;
use work.pk_pag.all;
entity tb_port_actual_generic_bounds is end entity;
architecture t of tb_port_actual_generic_bounds is
  constant r1 : rec := (1, 2, true);
  constant r2 : rec := (0, 0, false);
  signal o1, o2 : bit;
begin
  s1 : entity work.sub_pag generic map (r1) port map ("10", o1);
  s2 : entity work.sub_pag generic map (r2) port map ((others => '0'), o2);
end architecture;
