-- Foreign procedures and functions (VHPI models of plugins/fparam.c) with record, access and
-- signal parameters, one entity per case. Results land in signals the plugin traces. The
-- library word is the plugin's file name, as one reference simulator loads it from there.
library ieee;
use ieee.std_logic_1164.all;

entity fp_rec is end entity;

architecture sim of fp_rec is
  type rec is record
    i : integer;
    l : std_logic;
    v : std_logic_vector(3 downto 0);
  end record;
  procedure recp(r : in rec; o : out rec);
  attribute foreign of recp : procedure is "VHPI fparam recp";
  procedure recp(r : in rec; o : out rec) is begin end procedure;
  function recf(r : rec) return integer;
  attribute foreign of recf : function is "VHPI fparam recf";
  function recf(r : rec) return integer is begin return -1; end function;
  signal s_o : rec := (-9, 'U', "UUUU");
  signal s_f : integer := -9;
begin
  process
    variable o : rec := (0, '0', "0000");
  begin
    recp((41, '1', "0011"), o);
    s_o <= o;
    s_f <= recf((5, 'H', "1000"));
    wait;
  end process;
end architecture;

entity fp_acc is end entity;

architecture sim of fp_acc is
  type ip is access integer;
  procedure accp(variable p : in ip; r : out integer);
  attribute foreign of accp : procedure is "VHPI fparam accp";
  procedure accp(variable p : in ip; r : out integer) is begin end procedure;
  signal s_v : integer := -9;
  signal s_f : integer := -9;
  signal s_n : integer := -9;
begin
  process
    variable p : ip := new integer'(41);
    variable z : ip;
    variable r : integer := -1;
  begin
    accp(p, r);
    s_v <= p.all;
    s_f <= r;
    accp(z, r);
    s_n <= r;
    wait;
  end process;
end architecture;
