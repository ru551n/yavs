-- Cycle mode: combinational members the settle skips while the words they read are unchanged
-- (`codegen::Guard`). Each member is large enough for a guard of its own. `pc` counts the ones of
-- a wide input of which only bit 100 changes, at times back to its old value at the next wake
-- or in the next delta; `pf` and `pg` read one field each of a record whose other field
-- changes; `pm` reads a small memory a clocked member writes at run-time indices; `la` and `lb`
-- are a loop group whose second pass sees an input change; `pr` has an asynchronous reset from
-- outside pulsed between edges; `px` reads registers of two clocks (10 ns and 7 ns); `pd` reads
-- two inputs some words apart in the image that change together, one of them back a delta later.
-- With `FAIL`, an index check of `la` fails on an input only it reads, at a wake where its
-- early run (without checks) and its last run see the same inputs. `mon`, outside, prints every
-- output 1 ns after each edge of either clock.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_skip is
  generic (FAIL : boolean := false);
end entity;
architecture a of cycle_skip is
  type rec_t is record
    f1, f2 : std_ulogic_vector(7 downto 0);
  end record;
  type mem_t is array (0 to 15) of std_ulogic_vector(7 downto 0);
  signal clk, clk2, rst, done : std_ulogic := '0';
  signal w : std_ulogic_vector(127 downto 0) := (others => '0');
  signal cnt : unsigned(7 downto 0) := (others => '0');
  signal r : rec_t := (x"00", x"00");
  signal of1, of2 : std_ulogic_vector(7 downto 0) := (others => '0');
  signal mem : mem_t := (others => x"00");
  signal wa, ra : unsigned(3 downto 0) := (others => '0');
  signal wd, om : std_ulogic_vector(7 downto 0) := (others => '0');
  signal i1, i2, x1, x2, y1 : unsigned(7 downto 0) := (others => '0');
  signal q, oq : unsigned(7 downto 0) := (others => '0');
  signal r1, r2, ox : unsigned(7 downto 0) := (others => '0');
  signal da : std_ulogic_vector(7 downto 0) := x"00";
  signal pad : std_ulogic_vector(511 downto 0) := (others => '0');
  signal db : std_ulogic_vector(7 downto 0) := x"00";
  signal od, op : std_ulogic_vector(7 downto 0) := x"00";
  type tbl_t is array (0 to 7) of unsigned(7 downto 0);
  constant TBL : tbl_t := (x"11", x"22", x"33", x"44", x"55", x"66", x"77", x"88");
  signal i3 : natural range 0 to 15 := 0;
  signal z : unsigned(7 downto 0) := x"00";

  -- Many operations from a few input words: the guards' threshold.
  function mix(v : std_ulogic_vector(7 downto 0); k : natural) return std_ulogic_vector is
    variable x : std_ulogic_vector(7 downto 0) := v;
  begin
    for i in 0 to 23 loop
      x := (x(6 downto 0) & x(7)) xor std_ulogic_vector(to_unsigned((i * k + 3) mod 256, 8));
    end loop;
    return x;
  end function;
begin
  pc : process (w)
    variable n : unsigned(7 downto 0);
  begin
    n := (others => '0');
    for i in w'range loop
      if w(i) = '1' then
        n := n + i mod 7 + 1;
      end if;
    end loop;
    cnt <= n;
  end process;

  pf : process (r.f1)
  begin
    of1 <= mix(r.f1, 5);
  end process;

  pg : process (r.f2)
  begin
    of2 <= mix(r.f2, 11);
  end process;

  wr : process (clk)
  begin
    if rising_edge(clk) then
      mem(to_integer(wa)) <= wd;
    end if;
  end process;

  pm : process (mem, ra)
    variable x : std_ulogic_vector(7 downto 0);
  begin
    x := mem(to_integer(ra));
    for i in mem'range loop
      x := x xor mix(mem(i), i + 1);
    end loop;
    om <= x;
  end process;

  -- A loop group: `la` reads `x2`, which `lb` makes from `la`'s `x1`.
  la : process (i1, i2, i3, x2)
  begin
    x1 <= unsigned(mix(std_ulogic_vector(i1), 3));
    y1 <= unsigned(mix(std_ulogic_vector(x2 xor i2), 7));
    z <= TBL(i3);
  end process;

  lb : process (x1)
  begin
    x2 <= x1 + 1;
  end process;

  pr : process (clk, rst)
  begin
    if rst = '1' then
      q <= (others => '0');
    elsif rising_edge(clk) then
      q <= q + 3;
    end if;
  end process;

  po : process (q)
  begin
    oq <= unsigned(mix(std_ulogic_vector(q), 13));
  end process;

  c1 : process (clk)
  begin
    if rising_edge(clk) then
      r1 <= r1 + 1;
    end if;
  end process;

  c2 : process (clk2)
  begin
    if rising_edge(clk2) then
      r2 <= r2 + 5;
    end if;
  end process;

  px : process (r1, r2)
  begin
    ox <= unsigned(mix(std_ulogic_vector(r1 xor r2), 17));
  end process;

  pd : process (da, db)
  begin
    od <= mix(da xor db, 19);
  end process;

  pp : process (pad)
  begin
    op <= mix(pad(7 downto 0), 23);
  end process;

  gen1 : process
  begin
    for i in 1 to 30 loop
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      wait for 5 ns;
    end loop;
    done <= '1';
    wait;
  end process;

  gen2 : process
  begin
    while done = '0' loop
      clk2 <= '1';
      wait for 3 ns;
      clk2 <= '0';
      wait for 4 ns;
    end loop;
    wait;
  end process;

  stim : process
    variable va, vb : std_ulogic_vector(7 downto 0);
  begin
    for i in 1 to 30 loop
      wait until rising_edge(clk);
      wait for 2 ns;
      -- Only bit 100 of `w`: set, cleared at the next wake, or set and cleared a delta later.
      case i mod 4 is
        when 0 => w(100) <= '1';
        when 1 => w(100) <= '0';
        when 2 =>
          w(100) <= '1';
          wait for 0 ns;
          w(100) <= '0';
        when others => null;
      end case;
      -- One field of `r` at a time.
      if i mod 3 = 0 then
        r.f1 <= std_ulogic_vector(to_unsigned(i, 8));
      elsif i mod 3 = 1 then
        r.f2 <= std_ulogic_vector(to_unsigned(i * 7, 8));
      end if;
      wa <= to_unsigned(i mod 16, 4);
      wd <= std_ulogic_vector(to_unsigned(i * 9, 8));
      ra <= to_unsigned((i * 5) mod 16, 4);
      -- `i1` moves `x1`, `x2`: `la`'s second run sees `x2` change; `i2` alone only its first.
      if i mod 5 = 0 then
        i1 <= to_unsigned(i, 8);
      elsif i mod 5 = 2 then
        i2 <= to_unsigned(i, 8);
      end if;
      -- `da` and `db` together, and a delta later one of them back to what it was.
      if i mod 6 = 1 then
        vb := db;
        da <= std_ulogic_vector(to_unsigned(i, 8));
        db <= x"aa";
        wait for 0 ns;
        db <= vb;
      elsif i mod 6 = 4 then
        va := da;
        da <= x"55";
        db <= std_ulogic_vector(to_unsigned(i, 8));
        wait for 0 ns;
        da <= va;
      end if;
      -- `i3` alone: out of `TBL`'s range with `FAIL`.
      if i = 13 then
        i3 <= 9 when FAIL else 5;
      end if;
      -- The reset between edges, held over one edge in every seventh.
      if i mod 7 = 3 then
        rst <= '1';
        wait for 1 ns;
        rst <= '0';
      elsif i mod 7 = 5 then
        rst <= '1';
        wait until rising_edge(clk);
        wait for 1 ns;
        rst <= '0';
      end if;
    end loop;
    wait;
  end process;

  mon : process
  begin
    while done = '0' loop
      wait until rising_edge(clk) or rising_edge(clk2);
      wait for 1 ns;
      report "t=" & time'image(now) & " cnt=" & to_hstring(cnt) & " of1=" & to_hstring(of1)
        & " of2=" & to_hstring(of2) & " om=" & to_hstring(om) & " x1=" & to_hstring(x1)
        & " x2=" & to_hstring(x2) & " y1=" & to_hstring(y1) & " q=" & to_hstring(q)
        & " oq=" & to_hstring(oq) & " ox=" & to_hstring(ox) & " od=" & to_hstring(od)
        & " op=" & to_hstring(op) & " z=" & to_hstring(z);
    end loop;
    wait;
  end process;
end architecture;
