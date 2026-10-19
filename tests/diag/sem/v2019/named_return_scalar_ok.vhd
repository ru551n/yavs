-- yavs-test: std=2019
-- expect: none
-- LRM 2019 4.2.1 (lrm-checks L77): named return values of scalar types (integer, a subtype
-- with a range constraint, enumeration, physical, floating point) and of a protected method;
-- `r` takes the target's range
package p is
  subtype small is integer range 3 to 9;
  type len is range 0 to 1000 units mm; cm = 10 mm; end units;
  function f return r of integer;
  function g return r of small;
  function h return r of character;
  function l return r of len;
  function x return r of real;
  type store is protected
    impure function get return r of natural;
  end protected;
end package;
package body p is
  function f return r of integer is
    variable v : r := r'high;
  begin
    return v;
  end function;
  function g return r of small is
  begin
    return r'low when r'ascending;
    return r'high;
  end function;
  function h return r of character is
    variable v : r;
  begin
    return v;
  end function;
  function l return r of len is
  begin
    return r'right;
  end function;
  function x return r of real is
    variable v : r := r'low;
  begin
    return v;
  end function;
  type store is protected body
    impure function get return r of natural is
    begin
      return r'low;
    end function;
  end protected body;
end package body;
