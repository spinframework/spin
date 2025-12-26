#!/bin/bash

# Local PostgreSQL setup script for macOS
# Keeps data in ./pg directory

set -e  # Exit on error

# Configuration
PG_DIR="./pg"
PG_DATA="$PG_DIR/data"
PG_LOG="$PG_DIR/postgres.log"
DB_NAME="mydb"
PORT=5432

echo "🐘 PostgreSQL Local Setup"
echo "=========================="

# Check if PostgreSQL is installed
if ! command -v postgres &> /dev/null; then
    echo "❌ PostgreSQL is not installed."
    echo "Install it with: brew install postgresql@16"
    exit 1
fi

# Create pg directory if it doesn't exist
mkdir -p "$PG_DIR"

# Initialize database if data directory doesn't exist
if [ ! -d "$PG_DATA" ]; then
    echo "📁 Initializing new PostgreSQL database..."
    initdb -D "$PG_DATA" --encoding=UTF8 --locale=C
    
    echo "✅ Database initialized"
else
    echo "✅ Database already exists"
fi

# Check if PostgreSQL is already running
if pg_ctl -D "$PG_DATA" status &> /dev/null; then
    echo "⚠️  PostgreSQL is already running"
else
    # Start PostgreSQL
    echo "🚀 Starting PostgreSQL..."
    pg_ctl -D "$PG_DATA" -l "$PG_LOG" -o "-p $PORT" start
    
    # Wait for PostgreSQL to be ready
    echo "⏳ Waiting for PostgreSQL to be ready..."
    sleep 2
    
    # Check if server is ready
    until pg_isready -p $PORT &> /dev/null; do
        echo "   Still waiting..."
        sleep 1
    done
    
    echo "✅ PostgreSQL is running on port $PORT"
fi

# Check if database exists, create if not
if ! psql -p $PORT -lqt | cut -d \| -f 1 | grep -qw "$DB_NAME"; then
    echo "📊 Creating database '$DB_NAME'..."
    createdb -p $PORT "$DB_NAME"
    echo "✅ Database created"
else
    echo "✅ Database '$DB_NAME' already exists"
fi

# Check if users table exists
TABLE_EXISTS=$(psql -p $PORT -d "$DB_NAME" -tAc "SELECT EXISTS (SELECT FROM information_schema.tables WHERE table_name = 'users');")

if [ "$TABLE_EXISTS" = "f" ]; then
    echo "👥 Creating users table and adding sample data..."
    
    psql -p $PORT -d "$DB_NAME" << EOF
-- Create users table
CREATE TABLE users (
    id SERIAL PRIMARY KEY,
    name VARCHAR(100) NOT NULL,
    age INTEGER NOT NULL
);

-- Insert sample data
INSERT INTO users (name, age) VALUES
    ('Alice Johnson', 28),
    ('Bob Smith', 35),
    ('Charlie Brown', 42);

-- Display the data
SELECT * FROM users;
EOF
    
    echo "✅ Users table created and populated"
else
    echo "✅ Users table already exists"
    echo "📋 Current users:"
    psql -p $PORT -d "$DB_NAME" -c "SELECT * FROM users;"
fi

echo ""
echo "=========================="
echo "🎉 Setup complete!"
echo ""
echo "Connection details:"
echo "  Host: localhost"
echo "  Port: $PORT"
echo "  Database: $DB_NAME"
echo "  Data directory: $PG_DATA"
echo ""
echo "Useful commands:"
echo "  Connect: psql -p $PORT -d $DB_NAME"
echo "  Stop: pg_ctl -D $PG_DATA stop"
echo "  Status: pg_ctl -D $PG_DATA status"
echo "  View logs: tail -f $PG_LOG"