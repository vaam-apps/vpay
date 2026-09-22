## Default Permission

The two commands vpay's checkout window needs, and nothing else.

#### Granted Permissions

`allow-show` lets the front end open the payer's browser on a checkout
session's own hosted URL, and `allow-dismiss` lets it end that window. Neither
command can read a payment's outcome, reach vpay's merchant API, or open a URL
the caller did not supply, so this set is the whole of the plugin's surface —
an application that grants it grants no more than the ability to send the
payer to a checkout page.

#### This default permission set includes the following:

- `allow-show`
- `allow-dismiss`

## Permission Table

<table>
<tr>
<th>Identifier</th>
<th>Description</th>
</tr>


<tr>
<td>

`vpay-checkout:allow-dismiss`

</td>
<td>

Enables the dismiss command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`vpay-checkout:deny-dismiss`

</td>
<td>

Denies the dismiss command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`vpay-checkout:allow-show`

</td>
<td>

Enables the show command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`vpay-checkout:deny-show`

</td>
<td>

Denies the show command without any pre-configured scope.

</td>
</tr>
</table>
