-- difftest: top=tb_signal_alias_subprogram
-- Signal aliases (LRM 6.6.2) of a whole record signal and, inside a procedure, of a signal
-- parameter with a subtype indication of its own bounds; attributes of a rebased alias.
library ieee;
use ieee.std_logic_1164.all;
entity tb_signal_alias_subprogram is end entity;
architecture a of tb_signal_alias_subprogram is
  type rec_t is record
    x : integer;
    v : std_logic_vector(3 downto 0);
  end record;
  signal r : rec_t := (0, "0000");
  alias rr is r;
  signal v : std_logic_vector(7 downto 0) := x"a5";
  alias v_lo : std_logic_vector(0 to 3) is v(3 downto 0);
  procedure p(signal s : inout std_logic_vector) is
    alias t : std_logic_vector(1 to s'length) is s;
  begin
    t(1) <= '0';
    wait for 1 ns;
    report "p sees " & to_string(t) & " t'left=" & integer'image(t'left);
  end procedure;
begin
  process
  begin
    rr.x <= 5;
    rr.v <= "0010";
    wait for 1 ns;
    report "r.x=" & integer'image(r.x) & " r.v=" & to_string(rr.v);
    report "v_lo=" & to_string(v_lo) & " len=" & integer'image(v_lo'length)
      & " left=" & integer'image(v_lo'left) & " high=" & integer'image(v_lo'high);
    assert v_lo = "0101" severity failure;
    p(v_lo);
    report "v=" & to_string(v);
    wait;
  end process;
end architecture;
