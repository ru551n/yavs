-- difftest: top=logic_vector_ne std=2008
-- Predefined "=" and "/=" on std_logic_vector: "/=" must be the complement of "=" for
-- metavalues, differing lengths, null arrays, directions, slices, multi-word widths.
library ieee;
use ieee.std_logic_1164.all;
entity logic_vector_ne is end entity;
architecture a of logic_vector_ne is
  signal s1 : std_logic_vector(7 downto 0) := "UX-0LHZW";
  signal s2 : std_logic_vector(0 to 7) := "UX-0LHZW";
  signal s3 : std_logic_vector(7 downto 0) := "UX-0LHZ0";
begin
  process
    variable line_out : string(1 to 2);
    variable bad : natural := 0;
    variable res : string(1 to 64);
    variable n : natural := 0;
    procedure t(a, b : std_logic_vector) is
      variable e, ne : boolean;
    begin
      e := a = b; ne := a /= b;
      n := n + 1;
      res(n) := '0' when e else '1';
      if ne /= not e then bad := bad + 1;
        report "MISMATCH case " & integer'image(n) severity error; end if;
    end procedure;
    variable u, z, x1, x2, d, l, w65a, w65b, w129a, w129b : std_logic_vector(0 downto 0);
    variable a65, b65 : std_logic_vector(64 downto 0);
    variable a129, b129 : std_logic_vector(128 downto 0);
    variable nul : std_logic_vector(1 to 0);
    variable nul2 : std_logic_vector(0 downto 1);
  begin
    t("U", "0"); t("X", "X"); t("-", "0"); t("-", "-"); t("L", "0"); t("H", "1"); t("Z", "Z");
    t("W", "X"); t("U", "U"); t("01", "01"); t("01", "011"); t("011", "01"); t(nul, nul2);
    t(nul, "0"); t("0", nul);
    a65 := (others => '0'); b65 := (others => '0'); t(a65, b65);
    b65(64) := 'U'; t(a65, b65); b65(64) := '0'; b65(0) := 'X'; t(a65, b65);
    a129 := (others => '1'); b129 := (others => '1'); t(a129, b129);
    b129(64) := 'H'; t(a129, b129); b129(64) := '1'; b129(128) := '-'; t(a129, b129);
    t(a65(63 downto 0), b65(64 downto 1)); t(a65(64 downto 1), b65(64 downto 1));
    t(s1, s2); t(s1, s3); t(s1(3 downto 0), s3(3 downto 0)); t(s1(7 downto 1), s2(1 to 7));
    t(s1, "UX-0LHZW"); t(s1, "UX-0LHZ0");
    -- the same comparisons written inline (no procedure in between)
    n := n + 1; res(n) := '1' when s1 /= s3 else '0'; if (s1 /= s3) = (s1 = s3) then bad := bad + 1; end if;
    n := n + 1; res(n) := '1' when s1 /= s2 else '0'; if (s1 /= s2) = (s1 = s2) then bad := bad + 1; end if;
    n := n + 1; res(n) := '1' when b65 /= a65 else '0'; if (b65 /= a65) = (b65 = a65) then bad := bad + 1; end if;
    n := n + 1; res(n) := '1' when a65(63 downto 0) /= b65(63 downto 0) else '0';
    n := n + 1; res(n) := '1' when s1(7 downto 1) /= s3(6 downto 0) else '0';
    n := n + 1; res(n) := '1' when a129 /= b129 else '0'; if (a129 /= b129) = (a129 = b129) then bad := bad + 1; end if;
    -- non-logic element type: bit_vector (byte compare path)
    if (bit_vector'("0101") /= bit_vector'("0101")) or not (bit_vector'("011") /= bit_vector'("01")) then
      bad := bad + 1; report "bit_vector mismatch" severity error; end if;
    report "logic_vector_ne ne=" & res(1 to n) & " cases=" & integer'image(n) & " bad=" & integer'image(bad);
    assert bad = 0 report "/= is not the complement of =" severity failure;
    wait;
  end process;
end architecture;
