-- difftest: top=tb_signal_alias
-- Signal aliases (LRM 6.6.2): whole, slice, element, record element, with and without a
-- subtype indication, of ports and in processes: read, driven, 'event, 'last_value, waits.
library ieee;
use ieee.std_logic_1164.all;

entity sa_child is
  port (i : in std_logic_vector(7 downto 0); o : out std_logic_vector(3 downto 0));
end entity;

architecture a of sa_child is
  alias i_hi : std_logic_vector(3 downto 0) is i(7 downto 4);
  alias o_up : std_logic_vector(0 to 3) is o;
begin
  o_up <= i_hi;
end architecture;

library ieee;
use ieee.std_logic_1164.all;

entity tb_signal_alias is end entity;

architecture a of tb_signal_alias is
  type arr_t is array (natural range <>) of integer;
  type rec_t is record
    x : integer;
    v : std_logic_vector(3 downto 0);
  end record;
  signal s : arr_t(0 to 3) := (others => 0);
  signal b : bit := '0';
  signal v : std_logic_vector(7 downto 0) := x"00";
  signal r : rec_t := (0, "0000");
  signal q : std_logic_vector(3 downto 0);
  alias s_tail is s(2 to 3);
  alias s_re : arr_t(10 to 11) is s(0 to 1);
  alias s_el is s(1);
  alias b2 is b;
  alias v_lo : std_logic_vector(0 to 3) is v(3 downto 0);
  alias v_hi is v(7 downto 4);
  alias v_bit is v(5);
  alias r_v is r.v;
  alias r_x : integer is r.x;
  alias q2 is q;
  signal w : std_logic_vector(3 downto 0);
  alias w_up : std_logic_vector(0 to 1) is w(1 downto 0);
begin
  -- Each process drives only the elements it names through the rebased alias.
  wa : process begin w_up(0) <= '1'; wait; end process;
  wb : process begin w(0) <= '0'; wait; end process;
  wc : process begin w(3 downto 2) <= "01"; wait; end process;
  ww : process (w_up(0))
  begin
    report "w_up(0)=" & std_logic'image(w_up(0)) & " event=" & boolean'image(w_up(0)'event)
      & " w=" & to_string(w);
  end process;

  u : entity work.sa_child port map (i => v, o => q2);

  driver : process
  begin
    b2 <= '1';
    s_tail(2) <= 5;
    s_re(10) <= 7;
    s_el <= 3;
    v_lo <= "1000";
    v_hi(6) <= '1';
    r_v(2) <= '1';
    r_x <= 42;
    wait for 1 ns;
    v_hi <= "1010";
    s_tail <= (8, 9);
    wait for 1 ns;
    wait;
  end process;

  watch : process (v_bit)
  begin
    report "v_bit=" & std_logic'image(v_bit) & " event=" & boolean'image(v_bit'event)
      & " last=" & std_logic'image(v_bit'last_value);
  end process;

  check : process
    alias b3 is b;
    alias s3 : arr_t(1 downto 0) is s(2 to 3);
  begin
    wait on b3;
    report "b3 event=" & boolean'image(b3'event) & " last=" & bit'image(b3'last_value);
    wait for 0 ns;
    assert s_tail(2) = 5 and s(2) = 5 report "slice alias" severity failure;
    assert s_re(10) = 7 and s(0) = 7 and s_re(11) = 3 report "rebased alias" severity failure;
    assert s_el = 3 report "element alias" severity failure;
    assert s3(1) = 5 and s3(0) = 0 report "process alias" severity failure;
    assert w = "0110" report "rebased drivers got " & to_string(w) severity failure;
    assert v = "01001000" report "logic aliases" severity failure;
    assert v_lo(0) = '1' and v_lo'left = 0 report "logic rebased" severity failure;
    assert r_x = 42 and r_v = "0100" and r.v = "0100" report "record aliases" severity failure;
    wait until v_hi = "1010";
    wait for 0 ns;
    report "v_hi=" & to_string(v_hi) & " s3=" & integer'image(s3(1)) & integer'image(s3(0));
    assert q = "1010" report "port alias got " & to_string(q) severity failure;
    assert s_tail = (8, 9) report "aggregate to alias" severity failure;
    report "ok";
    wait;
  end process;
end architecture;
