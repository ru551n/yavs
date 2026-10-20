
package pkg_periodic_const is
  type slot_t is record
    id   : integer;
    mask : bit_vector(255 downto 0);
  end record;
  type slot_a is array (0 to 99) of slot_t;
  type bank_t is record
    bits  : bit_vector(4095 downto 0);
    slots : slot_a;
  end record;
  type bank_a is array (0 to 3) of bank_t;
  constant empty_slot : slot_t := (7, (others => '0'));
  constant empty_bank : bank_t := ((others => '1'), (others => empty_slot));
  constant banks : bank_a := (others => empty_bank);
end package;

package body pkg_periodic_const is
  function first_id return integer is
    constant c : bank_a := (others => empty_bank);
  begin
    return c(3).slots(99).id;
  end function;
end package body;
