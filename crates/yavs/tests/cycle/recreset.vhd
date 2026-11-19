-- Cycle mode gate design: arrays of records under reset. One table has a synchronous reset, one an
-- asynchronous reset, one a synchronous reset by a literal aggregate of the whole table (a process
-- that stays out of the region while a literal of records is not implemented). The first two are
-- written at a run-time index (a whole record or a field) while their other entries are updated
-- element by element in the same process. A combinational process counts the valid entries. The
-- monitor prints every table at each rising edge and 1 ns after it, so any difference shows.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_recreset is
end entity;
architecture a of cycle_recreset is
  type entry_t is record
    valid : std_ulogic;
    tag   : unsigned(3 downto 0);
    data  : unsigned(7 downto 0);
  end record;
  type table_t is array (0 to 3) of entry_t;
  signal clk, rst, arst, wr_en : std_ulogic := '0';
  signal wr_idx  : unsigned(1 downto 0) := "00";
  signal wr_data : unsigned(7 downto 0) := x"00";
  signal st, at, lt : table_t := (others => (valid => '0', tag => (others => '0'), data => (others => '0')));
  signal nvalid  : unsigned(2 downto 0) := "000";

  function one(e : entry_t) return string is
  begin
    return to_string(e.valid) & ":" & to_string(to_integer(e.tag)) & ":" & to_string(to_integer(e.data)) & " ";
  end function;
  function show(t : table_t) return string is
  begin
    return one(t(0)) & one(t(1)) & one(t(2)) & one(t(3));
  end function;
  impure function snap return string is
  begin
    return "st " & show(st) & "| at " & show(at) & "| lt " & show(lt) & "| n " & to_string(to_integer(nvalid));
  end function;
begin
  clk <= not clk after 5 ns;

  sync_table : process (clk)
  begin
    if rising_edge(clk) then
      if rst = '1' then
        for i in st'range loop
          st(i) <= (valid => '0', tag => (others => '0'), data => (others => '0'));
        end loop;
      else
        for i in st'range loop
          if st(i).valid = '1' then
            st(i).data <= st(i).data + 1;
          end if;
        end loop;
        if wr_en = '1' then
          st(to_integer(wr_idx)) <= (valid => '1', tag => resize(wr_idx, 4) + 1, data => wr_data);
        end if;
      end if;
    end if;
  end process;

  async_table : process (clk, arst)
  begin
    if arst = '1' then
      for i in at'range loop
        at(i) <= (valid => '0', tag => (others => '0'), data => (others => '0'));
      end loop;
    elsif rising_edge(clk) then
      if wr_en = '1' then
        at(to_integer(wr_idx)).valid <= '1';
        at(to_integer(wr_idx)).data <= wr_data;
      end if;
      for i in at'range loop
        at(i).tag <= at(i).tag + 1;
      end loop;
    end if;
  end process;

  -- a reset by a literal aggregate of the whole table (not implemented in cycle mode yet when this was written)
  lit_table : process (clk)
  begin
    if rising_edge(clk) then
      if rst = '1' then
        lt <= (others => (valid => '0', tag => (others => '0'), data => (others => '0')));
      elsif wr_en = '1' then
        lt(to_integer(wr_idx)).valid <= '1';
        lt(to_integer(wr_idx)).data <= wr_data + 3;
      end if;
    end if;
  end process;

  count : process (st)
    variable c : unsigned(2 downto 0);
  begin
    c := (others => '0');
    for i in st'range loop
      if st(i).valid = '1' then
        c := c + 1;
      end if;
    end loop;
    nvalid <= c;
  end process;

  -- two waits: stays outside the region; reads the values before the edge, then after it
  monitor : process
  begin
    wait until rising_edge(clk);
    report "pre  " & snap;
    wait for 1 ns;
    report "post " & snap;
  end process;

  stim : process
  begin
    rst <= '1';
    wait until falling_edge(clk);
    wait until falling_edge(clk);
    rst <= '0';
    for i in 0 to 29 loop
      wr_en <= '1' when i mod 3 /= 2 else '0';
      wr_idx <= to_unsigned((i * 5) mod 4, 2);
      wr_data <= to_unsigned(i * 7 + 1, 8);
      arst <= '1' when i = 11 or i = 12 or i = 25 else '0';
      rst <= '1' when i = 18 else '0';
      wait until falling_edge(clk);
    end loop;
    wr_en <= '0';
    wait until falling_edge(clk);
    wait for 1 ns;
    std.env.stop;
  end process;
end architecture;
