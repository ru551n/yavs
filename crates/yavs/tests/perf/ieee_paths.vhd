-- Every native ieee operation with a one-word or wide path, 30 times on 8, 32, 64, 65 and 128
-- elements, both directions, signed, integer, std_ulogic and Synopsys forms; every result goes
-- into the checksum. tests/perf.rs bounds the general paths and compiled code's native calls per key.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity ieee_paths_num is
  generic (W : positive);
end;
architecture a of ieee_paths_num is
  -- A shift that leaves at most 20 elements, and a modulus that fits the width.
  constant SH : natural := W - minimum(W, 20);
  constant M : positive := 2 ** minimum(W - 1, 20);
begin
  process
    variable a, b, bnz, r, s : unsigned(W - 1 downto 0);
    variable au, bu, bnzu : unsigned(0 to W - 1);
    variable sa, sb, sbnz, ss : signed(W - 1 downto 0);
    variable sau, sbu : signed(0 to W - 1);
    variable p : unsigned(2 * W - 1 downto 0);
    variable sp : signed(2 * W - 1 downto 0);
    variable v, vu : std_logic_vector(W - 1 downto 0);
    variable bv : bit_vector(W - 1 downto 0);
    variable acc, i, k : integer := 0;
    procedure mix(x : boolean) is
    begin
      if x then acc := (acc * 3 + 1) mod 1000003; else acc := (acc * 5 + 2) mod 1000003; end if;
    end procedure;
    procedure mix(x : std_ulogic) is
    begin
      mix(x = '1');
    end procedure;
  begin
    a := to_unsigned(16#3C#, W) + 7;
    b := to_unsigned(16#2E#, W) - 3;
    s := (others => '0');
    ss := (others => '0');
    for n in 1 to 30 loop
      i := n mod 13 + 1;
      k := n mod 5 + 1;
      a := rotate_left(a, 3) xor b;
      b := b + shift_right(a, 1) + 1;
      bnz := b or to_unsigned(1, W);
      au := a; bu := b; bnzu := bnz;
      sa := signed(a); sb := signed(b); sbnz := signed(bnz);
      sau := sa; sbu := sb;
      -- Arithmetic, both directions and both signednesses.
      s := s xor (a + b) xor (au + bu) xor (a - b) xor (au - bu) xor (a + i) xor (i + au) xor (a - i) xor (i - au);
      ss := ss xor (sa + sb) xor (sau - sbu) xor (sa + i) xor ((-i) + sau) xor (sa - i) xor (i - sau);
      ss := ss xor (-sa) xor (-sau) xor abs(sa) xor abs(sau);
      p := a * b; s := s xor p(W - 1 downto 0);
      p := au * bu; s := s xor p(W - 1 downto 0);
      sp := sa * sb; ss := ss xor sp(W - 1 downto 0);
      sp := sau * sbu; ss := ss xor sp(W - 1 downto 0);
      p := a * i; s := s xor p(W - 1 downto 0);
      p := i * au; s := s xor p(W - 1 downto 0);
      sp := sa * (-i); ss := ss xor sp(W - 1 downto 0);
      sp := i * sau; ss := ss xor sp(W - 1 downto 0);
      s := s xor (a / bnz) xor (au rem bnzu) xor (a mod bnz) xor (a / i) xor (au rem i) xor (a mod i);
      s := s xor resize(i / bnz, W) xor resize(i rem bnzu, W) xor resize(i mod bnz, W);
      ss := ss xor (sa / sbnz) xor (sau rem sbnz) xor (sa mod sbnz) xor (sa / (-i)) xor (sau rem i) xor (sa mod i);
      -- Relations, matching relations, maximum, minimum, std_match.
      mix(a < b); mix(au <= bu); mix(a > i); mix(i >= au); mix(a = b); mix(au /= i);
      mix(sa < sb); mix(sau >= sbu); mix(sa > (-i)); mix(i = sau); mix(sa /= sb);
      mix(a ?= b); mix(au ?/= bu); mix(a ?< b); mix(au ?>= i); mix(sa ?<= sb); mix(sau ?> (-i));
      s := s xor maximum(a, b) xor minimum(au, bu) xor maximum(a, i);
      ss := ss xor minimum(sa, sb) xor maximum(sau, sbu) xor minimum(sa, -i);
      mix(std_match(a, b)); mix(std_match(au, bu));
      -- Conversions, resizes, shifts and rotations.
      acc := (acc + to_integer(shift_right(a, SH)) + to_integer(shift_right(sau, SH))) mod 1000003;
      s := s xor to_unsigned(acc mod M, W) xor resize(resize(a, W / 2 + 3), W) xor resize(au, W) xor resize(resize(a, W + 9), W);
      ss := ss xor to_signed(-(acc mod M), W) xor resize(resize(sa, W / 2 + 3), W) xor resize(sau, W);
      s := s xor shift_left(a, k) xor shift_right(au, k) xor rotate_left(a, k) xor rotate_right(au, k);
      ss := ss xor shift_left(sa, k) xor shift_right(sau, k) xor rotate_right(sa, k);
      s := s xor (a sll k) xor (au srl k) xor (a rol k) xor (au ror k);
      -- Gates and to_01 / to_x01 / is_x / to_bitvector of vectors.
      s := s xor (a and b) xor (au or bu) xor (a nand b) xor (au nor bu) xor (a xnor b) xor (not au);
      ss := ss xor (sa and sb) xor (not sau);
      v := std_logic_vector(a) xor std_logic_vector(b);
      vu := (std_logic_vector(au) and std_logic_vector(bu)) or not std_logic_vector(a);
      s := s xor unsigned(v) xor unsigned(vu) xor to_01(a) xor to_01(au) xor unsigned(to_x01(v));
      mix(is_x(vu)); mix(xor a); mix(and sau);
      -- The predefined ?= and ?/= of std_ulogic_vector and std_ulogic.
      mix(v ?= vu); mix(std_logic_vector(au) ?/= std_logic_vector(bu)); mix(a(0) ?= b(k)); mix(au(1) ?/= bu(0));
      bv := to_bitvector(v); mix(bv(0) = '1');
      -- The gates of a vector and a scalar, and to_stdulogicvector.
      vu := (v and a(0)) xor (b(1) or vu); v := v xor to_stdulogicvector(bv);
      wait for 1 ns;
    end loop;
    report "W=" & integer'image(W) & " sum=" & to_hstring(s) & " " & to_hstring(ss) & " " & integer'image(acc);
    wait;
  end process;
end;

library ieee;
use ieee.std_logic_1164.all;
use ieee.std_logic_arith.all;
use ieee.std_logic_unsigned.all;
entity ieee_paths_syn is
  generic (W : positive);
end;
architecture a of ieee_paths_syn is
begin
  process
    variable ux, uy, ur : unsigned(W - 1 downto 0);
    variable sx, sy, sr : signed(W - 1 downto 0);
    variable up : unsigned(2 * W - 1 downto 0);
    variable sp : signed(2 * W - 1 downto 0);
    variable x, y, s : std_logic_vector(W - 1 downto 0);
    variable xp : std_logic_vector(2 * W - 1 downto 0);
    variable acc, i : integer := 0;
    variable b : std_ulogic;
    procedure mix(c : boolean) is
    begin
      if c then acc := (acc * 3 + 1) mod 1000003; else acc := (acc * 5 + 2) mod 1000003; end if;
    end procedure;
  begin
    ux := conv_unsigned(16#5A3C#, W);
    uy := conv_unsigned(16#1F2E#, W);
    s := (others => '0');
    for n in 1 to 30 loop
      i := n mod 13 + 1;
      ux := ux + uy + n;
      uy := uy - ux + 3;
      sx := signed(ux); sy := signed(uy);
      x := std_logic_vector(ux); y := std_logic_vector(uy);
      -- std_logic_arith on unsigned and signed, and the integer forms.
      ur := ux + uy; s := s xor std_logic_vector(ur);
      ur := ux - uy; s := s xor std_logic_vector(ur);
      ur := ux + i; s := s xor std_logic_vector(ur);
      ur := i - ux; s := s xor std_logic_vector(ur);
      sr := sx + sy; s := s xor std_logic_vector(sr);
      sr := sx - i; s := s xor std_logic_vector(sr);
      up := ux * uy; s := s xor std_logic_vector(up(W - 1 downto 0));
      sp := sx * sy; s := s xor std_logic_vector(sp(W - 1 downto 0));
      mix(ux < uy); mix(ux >= uy); mix(sx <= sy); mix(sx > i); mix(ux = uy); mix(sx /= sy);
      -- Conversions and shifts.
      acc := (acc + conv_integer(ux(19 mod W downto 0)) + conv_integer(sx(19 mod W downto 0))) mod 1000003;
      ur := conv_unsigned(sx, W); s := s xor std_logic_vector(ur);
      sr := conv_signed(i, W); s := s xor std_logic_vector(sr);
      s := s xor conv_std_logic_vector(acc, W) xor conv_std_logic_vector(ux, W);
      s := s xor ext(x(W - 1 downto W / 2), W) xor sxt(y(W / 2 downto 0), W);
      ur := shl(ux, conv_unsigned(i mod 7, 3)); s := s xor std_logic_vector(ur);
      sr := shr(sx, conv_unsigned(i mod 7, 3)); s := s xor std_logic_vector(sr);
      -- std_logic_unsigned on std_logic_vector, and std_logic_signed by expanded names.
      s := s xor (x + y) xor (x - y) xor (x + i) xor (i - y);
      xp := x * y; s := s xor xp(W - 1 downto 0);
      mix(x < y); mix(x = y); mix(x > i);
      s := s xor shl(x, "11") xor shr(y, "10");
      acc := (acc + conv_integer(x(19 mod W downto 0))) mod 1000003;
      s := s xor ieee.std_logic_signed."+"(x, y) xor ieee.std_logic_signed."-"(x, i);
      mix(ieee.std_logic_signed."<"(x, y));
      -- A std_ulogic operand of + and -, and the unary operators.
      b := ux(n mod W);
      ur := ux + b; s := s xor std_logic_vector(ur);
      ur := b - ux; s := s xor std_logic_vector(ur);
      sr := sx - b; s := s xor std_logic_vector(sr);
      sr := b + sx; s := s xor std_logic_vector(sr);
      s := s xor (x + b) xor (b - y) xor ieee.std_logic_signed."-"(x, b) xor ieee.std_logic_signed."+"(b, y);
      sr := -sx; s := s xor std_logic_vector(sr);
      sr := abs sy; s := s xor std_logic_vector(sr);
      s := s xor (+x) xor ieee.std_logic_signed."-"(x) xor ieee.std_logic_signed."abs"(y) xor ieee.std_logic_signed."+"(y);
      wait for 1 ns;
    end loop;
    report "W=" & integer'image(W) & " syn sum=" & to_hstring(s) & " " & integer'image(acc);
    wait;
  end process;
end;

entity ieee_paths is end;
architecture a of ieee_paths is
  type widths is array (natural range <>) of positive;
  constant WS : widths := (8, 32, 64, 65, 128);
begin
  g : for k in WS'range generate
    n : entity work.ieee_paths_num generic map (W => WS(k));
    s : entity work.ieee_paths_syn generic map (W => WS(k));
  end generate;
end;
