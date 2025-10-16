#!/bin/bash

# Master setup script for Molpha Oracle local testing environment
# This script will:
# 1. Start local validator
# 2. Deploy programs
# 3. Initialize accounts
# 4. Create test feeds
# 5. Set up test data for Go client

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Logging functions
log_info() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

log_success() {
    echo -e "${GREEN}[SUCCESS]${NC} $1"
}

log_warning() {
    echo -e "${YELLOW}[WARNING]${NC} $1"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

# Check if required tools are installed
check_prerequisites() {
    log_info "Checking prerequisites..."
    
    if ! command -v solana &> /dev/null; then
        log_error "Solana CLI not found. Please install it first."
        exit 1
    fi
    
    if ! command -v anchor &> /dev/null; then
        log_error "Anchor CLI not found. Please install it first."
        exit 1
    fi
    
    if ! command -v node &> /dev/null; then
        log_error "Node.js not found. Please install it first."
        exit 1
    fi
    
    log_success "All prerequisites found"
}

# Start local validator
start_validator() {
    log_info "Starting local Solana validator..."
    
    # Kill any existing validator
    pkill -f "solana-test-validator" || true
    sleep 2
    
    # Start validator in background
    solana-test-validator \
        --reset \
        --quiet \
        --bind-address 0.0.0.0 \
        --rpc-port 8899 \
        --faucet-port 9900 \
    
    VALIDATOR_PID=$!
    echo $VALIDATOR_PID > /tmp/solana-validator.pid
    
    # Wait for validator to start
    log_info "Waiting for validator to start..."
    for i in {1..30}; do
        if solana cluster-version &> /dev/null; then
            log_success "Validator started successfully"
            return 0
        fi
        sleep 1
    done
    
    log_error "Failed to start validator"
    exit 1
}

# Configure Solana CLI
configure_solana() {
    log_info "Configuring Solana CLI..."
    
    solana config set --url localhost
    solana config set --keypair ~/.config/solana/id.json
    
    # Create keypair if it doesn't exist
    if [ ! -f ~/.config/solana/id.json ]; then
        log_info "Creating new keypair..."
        solana-keygen new --no-bip39-passphrase --silent --outfile ~/.config/solana/id.json
    fi
    
    # Airdrop SOL
    log_info "Requesting airdrop..."
    solana airdrop 10 || log_warning "Airdrop failed, continuing anyway"
    
    WALLET_ADDRESS=$(solana address)
    log_success "Solana configured. Wallet: $WALLET_ADDRESS"
}

# Build and deploy programs
deploy_programs() {
    log_info "Building and deploying programs..."
    
    cd "$PROJECT_ROOT"
    
    # Build programs
    log_info "Building programs..."
    anchor build
    
    # Deploy programs
    log_info "Deploying programs..."
    anchor deploy
    
    log_success "Programs deployed successfully"
}

# Initialize the oracle system
initialize_system() {
    log_info "Initializing oracle system..."
    
    cd "$PROJECT_ROOT"
    
    # Initialize NodeRegistry
    log_info "Initializing NodeRegistry..."
    anchor run initialize || log_warning "NodeRegistry may already be initialized"
    
    # Initialize ProtocolConfig with 1000 lamports per update
    log_info "Initializing ProtocolConfig..."
    anchor run initialize-protocol -- 1000 || log_warning "ProtocolConfig may already be initialized"
    
    log_success "System initialized"
}

# Generate test keypairs and add nodes
setup_test_nodes() {
    log_info "Setting up test oracle nodes..."
    
    # Create test-keys directory
    mkdir -p "$PROJECT_ROOT/test-keys"
    
    # Generate 3 test node keypairs
    for i in {1..3}; do
        if [ ! -f "$PROJECT_ROOT/test-keys/node$i.json" ]; then
            log_info "Generating test node $i keypair..."
            solana-keygen new --no-bip39-passphrase --silent --outfile "$PROJECT_ROOT/test-keys/node$i.json"
        fi
        
        NODE_PUBKEY=$(solana-keygen pubkey "$PROJECT_ROOT/test-keys/node$i.json")
        log_info "Adding node $i to registry: $NODE_PUBKEY"
        
        # Add node to registry
        anchor run add-node -- "$NODE_PUBKEY" || log_warning "Node $i may already be registered"
    done
    
    log_success "Test nodes configured"
}

# Create test feeds
create_test_feeds() {
    log_info "Creating test data sources and feeds..."
    
    cd "$PROJECT_ROOT"
    
    # Create data sources first
    log_info "Creating public data source: TestDataSource"
    node scripts/create-data-source.js "TestDataSource" "public" || log_warning "Public data source may already exist"
    
    log_info "Creating personal data source: PersonalDataSource"
    node scripts/create-data-source.js "PersonalDataSource" "personal" || log_warning "Personal data source may already exist"
    
    # Create public feed (with 30 day subscription)
    log_info "Creating public feed: BTC-USD-PUBLIC"
    node scripts/create-feed.js "BTC-USD-PUBLIC" "TestDataSource" "public" 1 60 "QmTestCidPublic" 30 || log_warning "Public feed may already exist"
    
    # Create personal feed (with 30 day subscription)
    log_info "Creating personal feed: BTC-USD-PERSONAL"
    node scripts/create-feed.js "BTC-USD-PERSONAL" "PersonalDataSource" "personal" 2 30 "QmTestCidPersonal" 30 || log_warning "Personal feed may already exist"
    
    log_success "Test data sources and feeds created"
}

# Build Go client
build_go_client() {
    log_info "Building Go client..."
    
    cd "$PROJECT_ROOT/client"
    
    if [ ! -f go.mod ]; then
        log_error "Go client not found in client/ directory"
        return 1
    fi
    
    go mod tidy
    go build -o oracle-client main.go
    
    log_success "Go client built successfully"
}

# Generate test script
generate_test_script() {
    log_info "Generating test script..."
    
    cat > "$PROJECT_ROOT/test-go-client.sh" << 'EOF'
#!/bin/bash

# Test script for Go oracle client
# This script demonstrates how to use the Go client with the test environment

set -e

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CLIENT_DIR="$PROJECT_ROOT/client"

# Colors
GREEN='\033[0;32m'
BLUE='\033[0;34m'
NC='\033[0m'

log_info() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

log_success() {
    echo -e "${GREEN}[SUCCESS]${NC} $1"
}

cd "$CLIENT_DIR"

if [ ! -f oracle-client ]; then
    echo "Building Go client..."
    go build -o oracle-client main.go
fi

# Get wallet private key
WALLET_PRIVATE_KEY=$(cat ~/.config/solana/id.json | jq -r '[.[0:32]] | map(tostring) | join(",")')
WALLET_PRIVATE_KEY_BASE58=$(solana-keygen pubkey ~/.config/solana/id.json --outfile /dev/stdout | base58 -d | base58)

# Get test node private keys
NODE1_PRIVATE_KEY=$(cat ../test-keys/node1.json | jq -r '[.[0:32]] | map(tostring) | join(",")' | python3 -c "
import sys
import base58
data = list(map(int, sys.stdin.read().strip().split(',')))
print(base58.b58encode(bytes(data)).decode())
")

NODE2_PRIVATE_KEY=$(cat ../test-keys/node2.json | jq -r '[.[0:32]] | map(tostring) | join(",")' | python3 -c "
import sys
import base58
data = list(map(int, sys.stdin.read().strip().split(',')))
print(base58.b58encode(bytes(data)).decode())
")

log_info "Testing Go client..."

echo
log_info "1. Testing keypair generation..."
./oracle-client generate-keypair

echo
log_info "2. Testing public feed publication..."
./oracle-client publish "BTC-USD-PUBLIC" "50000" "$NODE1_PRIVATE_KEY" "$WALLET_PRIVATE_KEY_BASE58"

echo
log_info "3. Testing personal feed publication (single signature)..."
./oracle-client publish "BTC-USD-PERSONAL" "50100" "$NODE1_PRIVATE_KEY" "$WALLET_PRIVATE_KEY_BASE58"

echo
log_info "4. Testing personal feed publication (different node)..."
./oracle-client publish "BTC-USD-PERSONAL" "50200" "$NODE2_PRIVATE_KEY" "$WALLET_PRIVATE_KEY_BASE58"

echo
log_success "All tests completed successfully!"
log_info "Check the validator logs for transaction details:"
log_info "  tail -f /tmp/solana-validator.log"
EOF

    chmod +x "$PROJECT_ROOT/test-go-client.sh"
    log_success "Test script created: $PROJECT_ROOT/test-go-client.sh"
}

# Create monitoring script
create_monitoring_script() {
    log_info "Creating monitoring script..."
    
    cat > "$PROJECT_ROOT/monitor-oracle.sh" << 'EOF'
#!/bin/bash

# Oracle monitoring script
# Shows the current state of the oracle system

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Colors
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
NC='\033[0m'

echo -e "${BLUE}=== Molpha Oracle System Status ===${NC}"
echo

# Wallet info
WALLET_ADDRESS=$(solana address)
BALANCE=$(solana balance --lamports)
echo -e "${BLUE}Wallet:${NC} $WALLET_ADDRESS"
echo -e "${BLUE}Balance:${NC} $BALANCE lamports ($(echo "scale=4; $BALANCE / 1000000000" | bc) SOL)"
echo

# Program IDs
NODE_REGISTRY_PROGRAM=$(grep 'molpha_solana =' Anchor.toml | cut -d'"' -f2)
FEED_PROGRAM=$(grep 'molpha_feed =' Anchor.toml | cut -d'"' -f2)

echo -e "${BLUE}Program IDs:${NC}"
echo -e "  NodeRegistry: $NODE_REGISTRY_PROGRAM"
echo -e "  Feed:         $FEED_PROGRAM"
echo

# Check accounts
echo -e "${BLUE}Account Status:${NC}"

# NodeRegistry PDA
NODE_REGISTRY_PDA=$(solana address --keypair <(echo '[]') --program-id $NODE_REGISTRY_PROGRAM --seed node-registry 2>/dev/null || echo "Unable to derive")
if solana account $NODE_REGISTRY_PDA &>/dev/null; then
    echo -e "  ✅ NodeRegistry: $NODE_REGISTRY_PDA"
else
    echo -e "  ❌ NodeRegistry: Not initialized"
fi

# ProtocolConfig PDA  
PROTOCOL_CONFIG_PDA=$(solana address --keypair <(echo '[]') --program-id $FEED_PROGRAM --seed config 2>/dev/null || echo "Unable to derive")
if solana account $PROTOCOL_CONFIG_PDA &>/dev/null; then
    echo -e "  ✅ ProtocolConfig: $PROTOCOL_CONFIG_PDA"
else
    echo -e "  ❌ ProtocolConfig: Not initialized"
fi

echo

# Test nodes
echo -e "${BLUE}Test Nodes:${NC}"
for i in {1..3}; do
    if [ -f "test-keys/node$i.json" ]; then
        NODE_PUBKEY=$(solana-keygen pubkey "test-keys/node$i.json")
        echo -e "  Node $i: $NODE_PUBKEY"
    fi
done

echo
echo -e "${YELLOW}Commands:${NC}"
echo "  Monitor logs:     tail -f /tmp/solana-validator.log"
echo "  Test Go client:   ./test-go-client.sh"
echo "  Stop validator:   pkill -f solana-test-validator"
echo "  Account info:     solana account <address>"
EOF

    chmod +x "$PROJECT_ROOT/monitor-oracle.sh"
    log_success "Monitoring script created: $PROJECT_ROOT/monitor-oracle.sh"
}

# Create cleanup script
create_cleanup_script() {
    log_info "Creating cleanup script..."
    
    cat > "$PROJECT_ROOT/cleanup-test-env.sh" << 'EOF'
#!/bin/bash

# Cleanup script for test environment

echo "Cleaning up test environment..."

# Stop validator
echo "Stopping validator..."
pkill -f "solana-test-validator" || true

# Remove temporary files
echo "Removing temporary files..."
rm -rf /tmp/solana-ledger
rm -f /tmp/solana-validator.log
rm -f /tmp/solana-validator.pid

# Remove test keys (optional - uncomment if you want to remove them)
# echo "Removing test keys..."
# rm -rf test-keys/

echo "Cleanup completed!"
echo "Note: Your main Solana keypair (~/.config/solana/id.json) was not touched."
EOF

    chmod +x "$PROJECT_ROOT/cleanup-test-env.sh"
    log_success "Cleanup script created: $PROJECT_ROOT/cleanup-test-env.sh"
}

# Main execution
main() {
    echo -e "${BLUE}=== Molpha Oracle Local Environment Setup ===${NC}"
    echo
    
    check_prerequisites
    start_validator
    configure_solana
    deploy_programs
    initialize_system
    setup_test_nodes
    create_test_feeds
    build_go_client
    generate_test_script
    create_monitoring_script
    create_cleanup_script
    
    echo
    log_success "🎉 Local environment setup completed!"
    echo
    echo -e "${YELLOW}Next steps:${NC}"
    echo "1. Test the Go client:     ./test-go-client.sh"
    echo "2. Monitor the system:     ./monitor-oracle.sh"
    echo "3. View validator logs:    tail -f /tmp/solana-validator.log"
    echo "4. Cleanup when done:      ./cleanup-test-env.sh"
    echo
    echo -e "${BLUE}Test node keys are saved in:${NC} test-keys/"
    echo -e "${BLUE}Go client is built at:${NC} client/oracle-client"
    echo
}

# Handle script interruption
cleanup_on_exit() {
    log_warning "Script interrupted. Cleaning up..."
    pkill -f "solana-test-validator" || true
    exit 1
}

trap cleanup_on_exit INT TERM

# Run main function
main "$@" 