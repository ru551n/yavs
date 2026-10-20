package pkg_static_array is
end package;

package body pkg_static_array is
  function sum16(n : natural) return integer is
    type int_vec is array (0 to 15) of integer;
    variable v   : int_vec := (others => 1);
    variable acc : integer := 0;
  begin
    for k in 0 to n loop
      v(k mod 16) := v(k mod 16) + k;
      acc := acc + v((k * 7) mod 16);
    end loop;
    return acc;
  end function;
end package body;
