-- difftest: top=record_port_slice std=2008
-- An array of records passed as a port on a slice of a signal (`ch(0 to 39)`, `ch(1 to 40)`),
-- and each element of that port (`s(s'low + i)`) passed on as a record port of a generate lane:
-- forty chained stages on one array of records whose data field is 1312 elements long (over
-- 1024). Each lane runs 1100 activations, past the switch to its specialised process copy, and
-- checks what it reads ('event, 'last_value, the value) against what the stage before it drove.
library ieee;
use ieee.std_logic_1164.all;

package record_port_slice_pkg is
  type axis_t is record
    tvalid : std_logic;
    tdata  : std_logic_vector;
    tlast  : std_logic;
  end record;
  type axis_arr_t is array (natural range <>) of axis_t;
end package;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.record_port_slice_pkg.all;

entity record_port_slice_stage is
  port (clk : in std_logic; s : in axis_t; m : out axis_t);
end;
architecture t of record_port_slice_stage is
begin
  process (clk)
    variable prev : std_logic_vector(s.tdata'range) := (others => '0');
  begin
    if rising_edge(clk) then
      if s.tdata'event then
        assert false report "data changed with the clock" severity failure;
      end if;
      m.tvalid <= s.tvalid;
      m.tdata <= std_logic_vector(unsigned(s.tdata) + 1);
      m.tlast <= s.tvalid and not s.tlast;
    end if;
  end process;
  mon : process (s.tdata)
    variable seen : std_logic_vector(s.tdata'range) := (others => 'U');
  begin
    if now > 0 ns then
      assert s.tdata'event report "woken without an event" severity failure;
      assert s.tdata'last_value = seen report "wrong 'last_value" severity failure;
    end if;
    seen := s.tdata;
  end process;
end;

library ieee;
use ieee.std_logic_1164.all;
use work.record_port_slice_pkg.all;

entity record_port_slice_bank is
  port (clk : in std_logic; s : in axis_arr_t; m : out axis_arr_t);
end;
architecture t of record_port_slice_bank is
begin
  g : for i in 0 to s'length - 1 generate
    u : entity work.record_port_slice_stage port map (clk, s(s'low + i), m(m'low + i));
  end generate;
end;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.record_port_slice_pkg.all;

entity record_port_slice is end;
architecture t of record_port_slice is
  constant N : positive := 40;
  signal clk : std_logic := '0';
  signal ch : axis_arr_t(0 to N)(tdata(31 downto 0)) :=
    (others => (tvalid => '0', tdata => x"00000000", tlast => '0'));
begin
  b : entity work.record_port_slice_bank port map (clk, ch(0 to N - 1), ch(1 to N));

  process
    variable sum : unsigned(31 downto 0) := (others => '0');
    variable x : unsigned(31 downto 0) := x"0badf00d";
  begin
    for k in 1 to 1100 loop
      x := (x(30 downto 0) & (x(31) xor x(21) xor x(1) xor x(0)));
      ch(0).tvalid <= x(3);
      ch(0).tdata <= std_logic_vector(x);
      ch(0).tlast <= x(5);
      wait for 1 ns;
      clk <= '1'; wait for 1 ns;
      clk <= '0'; wait for 1 ns;
      -- The last stage drives 'U' until the first value has gone through all of them.
      if k > N then
        sum := (sum(26 downto 0) & sum(31 downto 27)) xor unsigned(ch(N).tdata) xor
          unsigned(ch(17).tdata);
        if ch(N).tvalid = '1' then sum := sum + 1; end if;
        if ch(N).tlast = '1' then sum := sum + 2; end if;
      end if;
      if k mod 275 = 0 then
        report "k=" & integer'image(k) & " sum=" & to_hstring(sum) & " ch(40)=" &
          to_hstring(ch(N).tdata) & " ch(17) last=" & to_hstring(ch(17).tdata'last_value);
      end if;
    end loop;
    wait;
  end process;
end;
