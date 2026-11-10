-- difftest: top=tb_fused_copy_address std=2008
-- Zero-delay drives of a whole logic value whose address is computed at run time (an element of
-- a constant of the instance, an element picked by a variable), on the way the kernel's fast
-- case declines: the first run of every process, a resolved target with two drivers, a driver
-- with a later transaction pending. Targets of 256, 40 and 32 elements that start inside a
-- plane word (fields of records in an array of records).
library ieee;
use ieee.std_logic_1164.all;
package fused_copy_address_p is
  subtype word is std_logic_vector(31 downto 0);
  type cfg_t is array (0 to 7) of word;
  type rec_t is record
    a : std_logic;
    cfg : cfg_t;
    n : std_logic_vector(39 downto 0);
    q : word;
    b : std_logic_vector(4 downto 0);
  end record;
  type recv_t is array (0 to 2) of rec_t;
  type tab_t is array (0 to 3) of word;
  function bits (n : natural) return word;
end package;

package body fused_copy_address_p is
  function bits (n : natural) return word is
    variable v : word := (others => '0');
  begin
    for i in 0 to 31 loop
      if (n / 2 ** (i mod 16)) mod 2 = 1 then
        v(i) := '1';
      end if;
    end loop;
    return v;
  end function;
end package body;

library ieee;
use ieee.std_logic_1164.all;
use work.fused_copy_address_p.all;
entity fused_copy_address_sub is
  generic (id : natural);
  port (o : out rec_t; tick : in natural);
end entity;
architecture a of fused_copy_address_sub is
  constant c : cfg_t := (0 => x"0100A5C3", 1 => bits(id), 4 => x"DEADBEEF",
                         7 => bits(id + 9), others => (others => '0'));
  constant cn : std_logic_vector(39 downto 0) := bits(id + 3) & x"5A";
  constant tab : tab_t := (bits(id), x"0000FFFF", bits(id * 5), x"12345678");
begin
  o.a <= '1';
  o.b <= "10101";
  o.cfg <= c;
  o.n <= cn;
  -- Two drivers of the resolved field: the other one drives 'Z' throughout.
  o.q <= (others => 'Z');
  process (tick)
    variable k : natural := 0;
  begin
    if tick mod 3 = 2 then
      -- A transaction 4 ns on, overwritten by the next zero-delay drive.
      o.q <= x"FFFFFFFF" after 4 ns;
    else
      o.q <= tab(k);
    end if;
    k := (k + 1) mod 4;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use work.fused_copy_address_p.all;
entity tb_fused_copy_address is end entity;
architecture t of tb_fused_copy_address is
  signal s : recv_t;
  signal tick : natural := 0;
begin
  g : for k in 0 to 2 generate
    u : entity work.fused_copy_address_sub generic map (id => k + 1) port map (o => s(k), tick => tick);
  end generate;

  process
    variable want : word;
  begin
    for t in 0 to 8 loop
      wait for 1 ns;
      for k in 0 to 2 loop
        assert s(k).a = '1' and s(k).b = "10101" severity failure;
        assert s(k).cfg(0) = x"0100A5C3" severity failure;
        assert s(k).cfg(1) = bits(k + 1) severity failure;
        assert s(k).cfg(4) = x"DEADBEEF" severity failure;
        assert s(k).cfg(7) = bits(k + 10) severity failure;
        for i in 2 to 6 loop
          assert i = 4 or s(k).cfg(i) = x"00000000" severity failure;
        end loop;
        assert s(k).n = bits(k + 4) & x"5A" severity failure;
        case t mod 4 is
          when 0 => want := bits(k + 1);
          when 1 => want := x"0000FFFF";
          when 2 => want := bits((k + 1) * 5);
          when others => want := x"12345678";
        end case;
        if t mod 3 /= 2 then
          assert s(k).q = want
            report "q of " & integer'image(k) & " at " & integer'image(t) & ": " & to_hstring(s(k).q)
            severity failure;
        end if;
        report integer'image(t) & " " & integer'image(k) & " " & to_hstring(s(k).cfg(1)) & " "
          & to_hstring(s(k).cfg(7)) & " " & to_hstring(s(k).n) & " " & to_hstring(s(k).q);
      end loop;
      tick <= t + 1;
    end loop;
    report "done";
    wait;
  end process;
end architecture;
