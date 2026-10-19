-- Foreign functions with array results (VHPI models of plugins/fresult.c), one entity per
-- case: unconstrained results sized with vhpiSizeConstraint or by the put itself, and a
-- constrained result. The results land in signals that the plugin reads at 5 ns. The
-- library word is the plugin's file name, as one reference simulator loads it from there.

library ieee;
use ieee.std_logic_1164.all;

entity fr_rev is end entity;

architecture sim of fr_rev is
  subtype nib is std_logic_vector(3 downto 0);
  function rev(v : std_logic_vector) return std_logic_vector;
  attribute foreign of rev : function is "VHPI fresult rev";
  function rev(v : std_logic_vector) return std_logic_vector is begin return v; end function;
  signal s_rev : std_logic_vector(5 downto 0) := (others => 'U');
  signal s_len : integer := -9;
  signal s_left, s_right : integer := -9;
begin
  process
  begin
    s_rev <= rev("110100");
    s_len <= rev("10")'length;
    s_left <= rev("101")'left;
    s_right <= rev("101")'right;
    wait;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;

entity fr_iota is end entity;

architecture sim of fr_iota is
  subtype nib is std_logic_vector(3 downto 0);
  function iota(n : integer) return integer_vector;
  attribute foreign of iota : function is "VHPI fresult iota";
  function iota(n : integer) return integer_vector is begin return (0 to n - 1 => -1); end function;
  signal s_iota : integer_vector(0 to 3) := (others => -9);
  signal s_left, s_right : integer := -9;
begin
  process
  begin
    s_iota <= iota(4);
    s_left <= iota(3)'left;
    s_right <= iota(3)'right;
    wait;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;

entity fr_word is end entity;

architecture sim of fr_word is
  subtype nib is std_logic_vector(3 downto 0);
  function word(n : integer) return string;
  attribute foreign of word : function is "VHPI fresult word";
  function word(n : integer) return string is begin return (1 to n => '?'); end function;
  signal s_word : string(1 to 3) := "...";
begin
  process
  begin
    s_word <= word(3);
    wait;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;

entity fr_nosize is end entity;

architecture sim of fr_nosize is
  subtype nib is std_logic_vector(3 downto 0);
  function nosize(n : integer) return integer_vector;
  attribute foreign of nosize : function is "VHPI fresult nosize";
  function nosize(n : integer) return integer_vector is begin return (0 to n - 1 => -1); end function;
  signal s_nosz : integer_vector(0 to 2) := (others => -9);
begin
  process
  begin
    s_nosz <= nosize(3);
    wait;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;

entity fr_fixed is end entity;

architecture sim of fr_fixed is
  subtype nib is std_logic_vector(3 downto 0);
  function fixed(x : integer) return nib;
  attribute foreign of fixed : function is "VHPI fresult fixed";
  function fixed(x : integer) return nib is begin return "UUUU"; end function;
  signal s_fixed : nib := "UUUU";
begin
  process
  begin
    s_fixed <= fixed(5);
    wait;
  end process;
end architecture;
