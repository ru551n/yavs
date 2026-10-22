-- difftest: top=tb_static_waits std=2008
-- Processes whose one wait names several signals (#530): a sensitivity list, `wait on`,
-- `wait until` over two signals, a concurrent assignment reading three, and a process that
-- also calls a procedure (no wait inside). Each must resume on every one of its signals.
library ieee;
use ieee.std_logic_1164.all;

entity tb_static_waits is end;

architecture t of tb_static_waits is
  type rec is record
    a : std_ulogic_vector(3 downto 0);
    b : integer;
  end record;
  type vecs is array (0 to 3) of std_ulogic_vector(1 downto 0);
  constant k : natural := 2;
  signal r : rec := ("0000", 0);
  signal v : vecs := (others => "00");
  signal x, y : std_ulogic := '0';
  signal o : std_ulogic_vector(3 downto 0);
  signal n_sens, n_on, n_until, n_call : natural := 0;

  procedure bump(signal c : inout natural) is
  begin
    c <= c + 1;
  end procedure;
begin
  sens : process (r, v(k), x)
  begin
    n_sens <= n_sens + 1;
    report "sens r.a=" & to_string(r.a) & " r.b=" & integer'image(r.b)
      & " v(k)=" & to_string(v(k)) & " x=" & std_ulogic'image(x);
  end process;

  on_list : process
  begin
    wait on x, y, r.b;
    n_on <= n_on + 1;
    report "on x=" & std_ulogic'image(x) & " y=" & std_ulogic'image(y) & " r.b=" & integer'image(r.b);
  end process;

  until_eq : process
  begin
    wait until x = y;
    n_until <= n_until + 1;
    report "until x=y=" & std_ulogic'image(x);
  end process;

  o <= r.a and (v(k) & x & y);

  calls : process (o, y)
  begin
    bump(n_call);
  end process;

  stim : process
  begin
    wait for 1 ns;
    r.a <= "1010";
    wait for 1 ns;
    v(k) <= "11";
    wait for 1 ns;
    v(1) <= "11";
    wait for 1 ns;
    x <= '1';
    wait for 1 ns;
    y <= '1';
    wait for 1 ns;
    r.b <= 4;
    wait for 1 ns;
    x <= '0';
    y <= '0';
    wait for 1 ns;
    report "o=" & to_string(o) & " sens=" & integer'image(n_sens) & " on=" & integer'image(n_on)
      & " until=" & integer'image(n_until) & " call=" & integer'image(n_call);
    wait;
  end process;
end;
