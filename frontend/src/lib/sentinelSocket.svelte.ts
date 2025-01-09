import type { AlertType, SentinelAlert } from './types';

export class SentinelSocket {
    socket?: WebSocket;
    alerts: SentinelAlert[] = $state<SentinelAlert[]>([]);
    connection: boolean = $state(false);
    adress: string = '';

    constructor(adress: string) {
        this.adress = adress;
    }

    connect() {
        this.socket = new WebSocket(this.adress);
        this.connection = true;
        this.socket.addEventListener('message', (event) => this.processMessage(event));
        this.socket.addEventListener('close', () => (this.connection = false));
        this.socket.addEventListener('error', (event) => this.handleError(event));
    }

    processMessage(event: MessageEvent) {
        let alert: SentinelAlert = JSON.parse(event.data);
        switch (alert.atype) {
            case 'New':
                this.alerts.push(alert);
                break;
            case 'Update':
                console.log('Update, TODO');
                break;
            case 'Remove':
                console.log('Remove, TODO');
                break;
            default:
                console.log(alert);
                break;
        }
    }

    close() {
        if (this.socket) {
            this.socket.close(1000);
        }
    }

    test() {
        setInterval(() => {
            console.log('Adding test alerts on interval');
            this.alerts = [
                {
                    id: 'id1',
                    atype: 'New' as AlertType,
                    name: 'test1',
                    performance: 500,
                    expected: 300,
                    reason: 'slow performance',
                    up: true
                },
                {
                    id: 'id2',
                    atype: 'New' as AlertType,
                    name: 'test2',
                    performance: 100,
                    expected: 100,
                    reason: 'No response, 404',
                    up: false
                },
                {
                    id: 'id3',
                    atype: 'New' as AlertType,
                    name: 'test3',
                    performance: 150,
                    expected: 150,
                    reason: 'Service Unaivalable, 503',
                    up: true
                },
                {
                    id: 'id4',
                    atype: 'New' as AlertType,
                    name: 'test4',
                    performance: 350,
                    expected: 100,
                    reason: 'slow performance',
                    up: true
                }
            ];
        }, 2000);
    }

    handleError(event: Event) {
        console.error('WebSocket error: ', event);
        this.connection = false;
        if (this.socket) {
            this.socket.close(3000, 'Client side error');
        }
    }
}
