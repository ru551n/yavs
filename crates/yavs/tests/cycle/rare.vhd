-- Reasons of constructs few designs use, one process each, with the reason (code and text)
-- the test expects: `rare_a_tb` the checker's and the region's, `rare_b_tb` the lowering's.
-- Analysed and run with `--relaxed` (shared variables of other than a protected type).
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
package rare_pkg is
  type ptr is access integer;
  impure function fnull return ptr;
  type prot is protected
    impure function get return integer;
  end protected;
  shared variable psv : integer := 0;
  procedure pause_if (go : boolean);
end package;

package body rare_pkg is
  impure function fnull return ptr is
  begin
    return null;
  end function;
  type prot is protected body
    variable n : integer := 0;
    impure function get return integer is
    begin
      n := n + 1;
      return n;
    end function;
  end protected body;
  procedure pause_if (go : boolean) is
  begin
    if go then
      wait for 1 ns;
    end if;
  end procedure;
end package body;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.rare_pkg.all;
entity rare_a is
  port (clk : in std_ulogic; a : in std_ulogic_vector(7 downto 0); ad : in unsigned(7 downto 0);
        i : in natural range 0 to 1; q : out std_ulogic_vector(7 downto 0));
end entity;
architecture rtl of rare_a is
  type ram_t is array (0 to 255) of std_ulogic_vector(31 downto 0);
  signal mem : ram_t;
  type iram_t is array (0 to 127) of integer;
  signal imem : iram_t := (others => 0);
  signal s1, s2, s3, s4, s5, s6, s7 : std_ulogic_vector(7 downto 0);
  signal m : std_ulogic_vector(31 downto 0);
  signal b1, b2, b3 : std_ulogic;
  signal clkv : std_ulogic_vector(1 downto 0);
  shared variable sv : integer;
  procedure pforce is
  begin
    b2 <= force '1';
  end procedure;
  procedure pdelay (signal s : out std_ulogic; v : std_ulogic) is
  begin
    s <= v after 1 ns;
  end procedure;
  function fedge (signal c : std_ulogic) return boolean is
  begin
    return c'event and c = '1';
  end function;
  impure function fprot return integer is
    variable lp : prot;
  begin
    return lp.get;
  end function;
begin
  implicit : process (clk)
  begin
    if rising_edge(clk) then
      b1 <= a(0)'delayed(0 ns);
    end if;
  end process;
  g : block (a(0) = '1')
  begin
    guard_read : process (clk)
    begin
      if rising_edge(clk) then
        if GUARD then
          s1 <= a;
        end if;
      end if;
    end process;
  end block;
  shared_comb : process (a)
  begin
    sv := to_integer(unsigned(a));
    s2 <= std_ulogic_vector(to_unsigned(sv, 8));
  end process;
  access_val : process (clk)
  begin
    if rising_edge(clk) then
      s3 <= std_ulogic_vector(to_unsigned(integer(fnull.all), 8));
    end if;
  end process;
  clock_at_index : process (clkv, i)
  begin
    if clkv(i)'event and clkv(i) = '1' then
      s4 <= a;
    end if;
  end process;
  call_force : process (clk)
  begin
    if rising_edge(clk) then
      pforce;
    end if;
  end process;
  call_delay : process (a)
  begin
    pdelay(b3, a(1));
  end process;
  call_edge : process (clk, a)
  begin
    if fedge(clk) then
      s5 <= a;
    end if;
  end process;
  call_prot : process (clk)
  begin
    if rising_edge(clk) then
      s6 <= std_ulogic_vector(to_unsigned(fprot, 8));
    end if;
  end process;
  mem_both : process (clk)
  begin
    if rising_edge(clk) then
      mem(to_integer(ad)) <= a & a & a & a;
      mem(0) <= (others => '0');
      m <= mem(to_integer(ad));
    end if;
  end process;
  mem_out : process (clk)
  begin
    if rising_edge(clk) then
      imem(to_integer(ad(6 downto 0))) <= to_integer(unsigned(a));
    end if;
  end process;
  call_wait : process (clk)
  begin
    if rising_edge(clk) then
      pause_if(false);
    end if;
  end process;
  reader : process
  begin
    wait for 10 ns;
    s7 <= std_ulogic_vector(to_unsigned(imem(0), 8));
  end process;
  clkv <= clk & clk;
  q <= s1 xor s2 xor s3 xor s4 xor s5 xor s6 xor s7 xor m(7 downto 0);
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.rare_pkg.all;
entity rare_b is
  port (clk : in std_ulogic; a : in std_ulogic_vector(7 downto 0); ad : in unsigned(7 downto 0);
        q : out std_ulogic_vector(7 downto 0));
end entity;
architecture rtl of rare_b is
  type a2 is array (natural range <>) of std_ulogic_vector(1 downto 0);
  type a3 is array (natural range <>) of a2(0 to 1);
  type a4 is array (natural range <>) of a3(0 to 1);
  type a5 is array (natural range <>) of a4(0 to 1);
  type ia is array (0 to 3) of integer;
  type ib is array (0 to 3) of natural;
  type st_t is (st0, st1, st2, st3);
  signal sel : std_ulogic;
  type out_t is array (1 to 40) of std_ulogic_vector(7 downto 0);
  signal ob : out_t;
  signal x01s : X01;
  signal n5 : a5(0 to 1);
  signal wide : std_ulogic_vector(71 downto 0);
  signal st : st_t;
  function noret (x : std_ulogic) return std_ulogic is
  begin
    if x = '1' then
      return '0';
    end if;
  end function;
  function small (n : natural range 0 to 15) return std_ulogic_vector is
  begin
    return std_ulogic_vector(to_unsigned(n, 8));
  end function;
  function fix8 (x : std_ulogic_vector(7 downto 0)) return std_ulogic_vector is
  begin
    return x;
  end function;
  function half (x : std_ulogic_vector) return std_ulogic_vector is
  begin
    return x(x'left downto x'length / 2);
  end function;
  procedure pind (x : std_ulogic_vector(1 downto 0); signal o : out std_ulogic_vector) is
  begin
    o <= x & x & x & x;
  end procedure;
  function fact (n : natural) return natural is
  begin
    if n = 0 then
      return 1;
    end if;
    return (n * fact(n - 1)) mod 256;
  end function;
  function odd (n : natural) return std_ulogic is
  begin
    assert n mod 2 = 1 report "even" severity note;
    return '1';
  end function;
begin
  val_logic : process (clk)
  begin
    if rising_edge(clk) then
      ob(1) <= (others => std_ulogic'val(to_integer(ad(2 downto 0))));
    end if;
  end process;
  int_wide : process (clk)
  begin
    if rising_edge(clk) then
      ob(2) <= std_ulogic_vector(to_unsigned(to_integer(unsigned(wide)) mod 256, 8));
    end if;
  end process;
  wide <= a & a & a & a & a & a & a & a & a;
  x01_sig : process (clk)
  begin
    if rising_edge(clk) then
      x01s <= to_x01(a(0));
    end if;
  end process;
  deep : process (clk)
  begin
    if rising_edge(clk) then
      n5(0)(0)(0)(0) <= a(1 downto 0);
    end if;
  end process;
  int_conv : process (clk)
    variable va : ia;
    variable vb : ib;
  begin
    if rising_edge(clk) then
      va := (1, 2, 3, to_integer(ad(1 downto 0)));
      vb := ib(va);
      ob(5) <= std_ulogic_vector(to_unsigned(vb(3), 8));
    end if;
  end process;
  no_return : process (clk)
  begin
    if rising_edge(clk) then
      ob(6) <= (others => noret(a(0)));
    end if;
  end process;
  outside_formal : process (clk)
  begin
    if rising_edge(clk) then
      ob(7) <= small(20);
    end if;
  end process;
  individual : process (clk)
  begin
    if rising_edge(clk) then
      pind(x(1) => a(0), x(0) => a(1), o => ob(8));
    end if;
  end process;
  succ : process (clk)
  begin
    if rising_edge(clk) then
      st <= st_t'succ(st) when st /= st3 else st0;
    end if;
  end process;
  pkg_shared : process (clk)
  begin
    if rising_edge(clk) then
      psv := psv + 1;
      ob(10) <= std_ulogic_vector(to_unsigned(psv mod 256, 8));
    end if;
  end process;
  recursion : process (clk)
  begin
    if rising_edge(clk) then
      ob(11) <= std_ulogic_vector(to_unsigned(fact(to_integer(ad(2 downto 0))), 8));
    end if;
  end process;
  short_value : process (clk)
  begin
    if rising_edge(clk) then
      ob(12) <= half(a);
    end if;
  end process;
  short_gate : process (clk)
  begin
    if rising_edge(clk) then
      ob(13) <= a and half(a) & half(a) & half(a);
    end if;
  end process;
  short_match : process (clk)
  begin
    if rising_edge(clk) then
      ob(14) <= (others => a ?= half(a));
    end if;
  end process;
  short_formal : process (clk)
  begin
    if rising_edge(clk) then
      ob(15) <= fix8(half(a) & "00");
    end if;
  end process;
  slice_dir : process (clk)
  begin
    if rising_edge(clk) then
      ob(16)(3 downto 0) <= a(0 to 3);
    end if;
  end process;
  slice_out : process (clk)
  begin
    if rising_edge(clk) then
      ob(17)(3 downto 0) <= a(9 downto 6);
    end if;
  end process;
  huge : process (clk)
    variable big : std_ulogic_vector(2**27 - 1 downto 0);
  begin
    if rising_edge(clk) then
      big(0) := a(0);
      ob(18)(0) <= big(0);
    end if;
  end process;
  neg_count : process (clk)
    variable n : integer range -3 to 3;
  begin
    if rising_edge(clk) then
      n := to_integer(signed(ad(2 downto 0)));
      ob(19) <= std_ulogic_vector(unsigned(a) sll n);
    end if;
  end process;
  two_conds : process (clk)
  begin
    if rising_edge(clk) and a(0) = '1' and a(1) = '1' then
      ob(20) <= a;
    end if;
  end process;
  known_assert : process (clk)
    variable v : std_ulogic_vector(7 downto 0);
  begin
    if rising_edge(clk) then
      for i in 0 to 7 loop
        v(i) := odd(i) and a(i);
      end loop;
      ob(21) <= v;
    end if;
  end process;
  pieces : process (clk)
  begin
    if rising_edge(clk) then
      ob(22) <= (a(3 downto 0), a(7 downto 4));
    end if;
  end process;
  null_size : process (clk)
  begin
    if rising_edge(clk) then
      ob(23) <= a(7 downto 0) & std_ulogic_vector(resize(unsigned(a), 0));
    end if;
  end process;
  neg_known : process (clk)
  begin
    if rising_edge(clk) then
      ob(24) <= std_ulogic_vector(shift_left(unsigned(a), -1));
    end if;
  end process;
  choice_len : process (clk)
  begin
    if rising_edge(clk) then
      case std_ulogic_vector'("01") is
        when "011" => ob(25) <= a;
        when others => ob(25) <= not a;
      end case;
    end if;
  end process;
  with unsigned(a) select sel <= '1' when to_unsigned(42, 8), '0' when others;
  q <= ob(1);
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity rare_a_tb is
end entity;
architecture tb of rare_a_tb is
  signal clk : std_ulogic := '0';
  signal a, q : std_ulogic_vector(7 downto 0);
  signal ad : unsigned(7 downto 0);
  signal i : natural range 0 to 1;
begin
  dut : entity work.rare_a port map (clk, a, ad, i, q);
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
-- The reports of a clocked checker of the top-level unit that region code cannot evaluate.
entity rare_b_tb is
end entity;
architecture tb of rare_b_tb is
  signal clk : std_ulogic := '0';
  signal a, q : std_ulogic_vector(7 downto 0);
  signal ad : unsigned(7 downto 0);
  signal sev : severity_level := note;
begin
  dut : entity work.rare_b port map (clk, a, ad, q);
  sev_signal : process (clk)
  begin
    if rising_edge(clk) then
      report "x" severity sev;
    end if;
  end process;
  real_image : process (clk)
  begin
    if rising_edge(clk) then
      report "r=" & real'image(1.5);
    end if;
  end process;
  hex_signed : process (clk)
  begin
    if rising_edge(clk) then
      report "h=" & to_hstring(signed(a));
    end if;
  end process;
end architecture;
