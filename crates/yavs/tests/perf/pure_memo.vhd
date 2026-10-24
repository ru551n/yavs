-- Pure functions on unconstrained vectors (the shape of PoC ffdre, resize and mux) called with
-- repeating arguments from clocked statements: the memo of pure calls (#701) answers the repeats.
library ieee; use ieee.std_logic_1164.all;
package pure_memo_pkg is
  constant SIMULATION : boolean := true;
  function imin(a : integer; b : integer) return integer;
  function resize(vec : std_logic_vector; length : natural; fill : std_logic := '0') return std_logic_vector;
  function ffdre(q : std_logic; d : std_logic; rst : std_logic := '0'; en : std_logic := '1'; constant INIT : std_logic := '0') return std_logic;
  function ffdre(q : std_logic_vector; d : std_logic_vector; rst : std_logic := '0'; en : std_logic := '1'; constant INIT : std_logic_vector := (0 to 0 => '0')) return std_logic_vector;
  function mux(sel : std_logic; slv0 : std_logic_vector; slv1 : std_logic_vector) return std_logic_vector;
end package;
package body pure_memo_pkg is
  function imin(a : integer; b : integer) return integer is
  begin
    if a < b then return a; else return b; end if;
  end function;
  function resize(vec : std_logic_vector; length : natural; fill : std_logic := '0') return std_logic_vector is
    constant high2b : natural := vec'low + length - 1;
    constant highcp : natural := imin(vec'high, high2b);
    variable res_up : std_logic_vector(vec'low to high2b);
    variable res_dn : std_logic_vector(high2b downto vec'low);
  begin
    if vec'ascending then
      res_up := (others => fill);
      res_up(vec'low to highcp) := vec(vec'low to highcp);
      return res_up;
    else
      res_dn := (others => fill);
      res_dn(highcp downto vec'low) := vec(highcp downto vec'low);
      return res_dn;
    end if;
  end function;
  function ffdre(q : std_logic; d : std_logic; rst : std_logic := '0'; en : std_logic := '1'; constant INIT : std_logic := '0') return std_logic is
  begin
    if not SIMULATION then
      if INIT = '0' then
        return ((d and en) or (q and not en)) and not rst;
      elsif INIT = '1' then
        return ((d and en) or (q and not en)) or rst;
      else
        report "Unsupported INIT value for synthesis." severity failure;
        return 'X';
      end if;
    elsif rst = '1' then
      return INIT;
    else
      return ((d and en) or (q and not en));
    end if;
  end function;
  function ffdre(q : std_logic_vector; d : std_logic_vector; rst : std_logic := '0'; en : std_logic := '1'; constant INIT : std_logic_vector := (0 to 0 => '0')) return std_logic_vector is
    constant INIT_I : std_logic_vector(q'range) := resize(INIT, q'length);
    variable Result : std_logic_vector(q'range);
  begin
    for i in q'range loop
      Result(i) := ffdre(q => q(i), d => d(i), rst => rst, en => en, INIT => INIT_I(i));
    end loop;
    return Result;
  end function;
  function mux(sel : std_logic; slv0 : std_logic_vector; slv1 : std_logic_vector) return std_logic_vector is
  begin
    return (slv0 and not (slv0'range => sel)) or (slv1 and (slv1'range => sel));
  end function;
end package body;

library ieee; use ieee.std_logic_1164.all; use work.pure_memo_pkg.all;
entity pure_memo is end;
architecture a of pure_memo is
  constant W : natural := 2;
  constant INITIAL : std_logic_vector(W-1 downto 0) := "10";
  signal clk : std_logic := '0';
  signal rst : std_logic := '1';
  signal q, nxt, m : std_logic_vector(W-1 downto 0) := "00";
  signal sel : std_logic := '0';
  signal done : boolean := false;
  signal n : natural := 0;
begin
  clk <= not clk after 5 ns when not done else clk;
  nxt <= q(0) & q(1) when n mod 3 = 0 else not q;
  q <= ffdre(q => q, d => nxt, rst => rst, INIT => INITIAL) when rising_edge(clk);
  m <= mux(sel, q, nxt);
  process (clk) begin
    if rising_edge(clk) then
      n <= n + 1;
      sel <= not sel;
      if n = 4 then rst <= '0'; end if;
    end if;
  end process;
  process
    variable s : natural := 0;
  begin
    for c in 1 to 150 loop wait until rising_edge(clk); end loop;
    wait for 1 ns;
    for i in 0 to W-1 loop
      if q(i) = '1' then s := s + 2 ** i; end if;
      if m(i) = '1' then s := s + 8 * 2 ** i; end if;
    end loop;
    done <= true;
    report "q=" & integer'image(s);
    wait;
  end process;
end;
