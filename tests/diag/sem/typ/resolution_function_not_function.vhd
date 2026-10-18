-- yavs-test: std=2008
-- expect: E1225 6:19 `res` is not a function
-- LRM 4.6
package p is
  constant res : integer := 1;
  subtype rbit is res bit;
end package;
