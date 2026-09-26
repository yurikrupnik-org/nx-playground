"""Hello unit test module."""

from test_cli1.hello import hello


def test_hello():
    """Test the hello function."""
    assert hello() == "Hello test-cli1"
