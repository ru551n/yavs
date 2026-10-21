-- yavs-test: std=2008
-- expect: none
-- LRM 9.4.3, 6.6.2 (OSVVM AxiStreamReceiver: constants from 'length of an unconstrained port are globally static; sim1, sim2 and sim3 accept)
library ieee; use ieee.std_logic_1164.all;
entity e is
  port (TID : in std_logic_vector);
end entity;
architecture a of e is
  constant ID_LEN : integer := TID'length;
  constant PARAM_LENGTH : integer := ID_LEN + 1;
begin
  process
    function f (Param : std_logic_vector) return string is
      alias aParam : std_logic_vector(Param'length-1 downto 0) is Param;
      alias ID : std_logic_vector(ID_LEN-1 downto 0) is aParam(PARAM_LENGTH-1 downto PARAM_LENGTH-ID_LEN);
    begin
      return to_string(ID);
    end function;
  begin
    wait;
  end process;
end architecture;
